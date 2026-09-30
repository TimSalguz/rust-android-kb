//! `kbime` — the Android input method, in Rust.
//!
//! Android requires the IME to be a Java `InputMethodService`; ours
//! (`android/java/…/RustKbService.java`) is a thin shim that forwards touches
//! and editor events here and paints the draw list we return. Everything else —
//! layouts, touch handling, composing, autocorrect, rendering decisions — lives
//! in [`ime::Ime`], which is plain Rust and tested on the desktop.
//!
//! Every JNI entry point catches panics (release builds unwind) so a bug
//! degrades to a no-op instead of killing the keyboard process.

pub mod grip;
pub mod i18n;
pub mod ime;
pub mod layout;
pub mod panel;
pub mod settings;

use std::panic::{catch_unwind, AssertUnwindSafe};

use jni::objects::{JClass, JIntArray, JObject, JString};
use jni::sys::{jboolean, jfloat, jint, jintArray, jlong, jobjectArray, jstring};
use jni::JNIEnv;
use kbcore::{Config, Engine, Mmap, Profile};

use crate::ime::{Ime, DOWN, REDRAW, RELAYOUT};
use crate::layout::Lang;
use crate::settings::{Settings, SettingsFile};

struct Handle {
    ime: Ime<Mmap>,
    /// The calibrated grips, next to the settings.
    grip_path: std::path::PathBuf,
    /// The user's own words (added by hand), next to the settings.
    words_path: std::path::PathBuf,
    /// Where debug log lines go (appended), if the shim gave a path.
    log_path: Option<std::path::PathBuf>,
    /// Texts of the last draw list (`drawOps` builds it, `drawTexts` hands it over).
    texts: Vec<String>,
    settings: SettingsFile,
}

impl Handle {
    /// Append pending debug log lines to the log file (capped at ~20 MB: the
    /// previous file is kept as `.old`).
    fn flush_log(&mut self) {
        let lines = self.ime.take_log();
        let Some(path) = &self.log_path else { return };
        if lines.is_empty() {
            return;
        }
        use std::io::Write;
        if std::fs::metadata(path).is_ok_and(|m| m.len() > 20 << 20) {
            let _ = std::fs::rename(path, path.with_extension("jsonl.old"));
        }
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            for l in lines {
                let _ = writeln!(f, "{l}");
            }
        }
    }

    /// Pick up changes made on the settings screen.
    fn save_words(&mut self) {
        if let Some(text) = self.ime.take_user_words() {
            let tmp = self.words_path.with_extension("tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(tmp, &self.words_path);
            }
        }
    }

    fn save_grips(&mut self) {
        if let Some(text) = self.ime.take_grips() {
            let tmp = self.grip_path.with_extension("tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(tmp, &self.grip_path);
            }
        }
    }

    /// Re-read the settings file if it changed; whether it did.
    fn reload_settings(&mut self) -> bool {
        match self.settings.changed() {
            Some(s) => {
                self.ime.set_settings(s);
                true
            }
            None => false,
        }
    }

    /// PACKS when the enabled languages need data packs not open yet.
    fn packs_flag(&self) -> jint {
        if self.ime.wanted_packs().is_empty() {
            0
        } else {
            PACKS
        }
    }
}

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Response flag (next to ime's REDRAW, OUTPUT, …): install the packs of
/// `wantedPacks` and hand them over with `addPack`.
const PACKS: jint = 16;

/// The engine config on a phone for a language pack's layout.
fn phone_config(latin: kbcore::Latin) -> Config {
    // The home-row reading is a hardware-keyboard idea; on a phone the
    // one-row layout (a setting) replaces it.
    Config {
        home_row: false,
        latin,
        ..Config::balanced().with_profile(Profile::Phone)
    }
}

/// Open a dictionary with its optional context model and capitals list.
fn open_engine(
    dict: &str,
    bigrams: Option<String>,
    casing: Option<String>,
    cfg: Config,
) -> Result<Engine<kbcore::Mmap>> {
    let mut engine = match bigrams {
        Some(b) => Engine::open_with_bigrams(dict, b, cfg)?,
        None => Engine::open(dict, cfg)?,
    };
    if let Some(c) = casing {
        engine = engine.open_casing(c)?;
    }
    Ok(engine)
}

/// A nullable Java string.
fn opt_string(env: &mut JNIEnv, s: &JString) -> Result<Option<String>> {
    Ok(if s.is_null() {
        None
    } else {
        Some(env.get_string(s)?.into())
    })
}

#[cfg(target_os = "android")]
fn log(msg: &str) {
    use std::os::raw::{c_char, c_int};
    #[link(name = "log")]
    extern "C" {
        fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
    }
    const ANDROID_LOG_ERROR: c_int = 6;
    let text = std::ffi::CString::new(msg.replace('\0', " ")).unwrap_or_default();
    // SAFETY: both pointers are valid NUL-terminated strings for the call's duration.
    unsafe {
        __android_log_write(ANDROID_LOG_ERROR, c"rustkb".as_ptr(), text.as_ptr());
    }
}

#[cfg(not(target_os = "android"))]
fn log(msg: &str) {
    eprintln!("rustkb: {msg}");
}

/// Run `f`, turning errors and panics into `fallback` (and a log line).
fn guard<T>(fallback: T, f: impl FnOnce() -> Result<T>) -> T {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            log(&format!("error: {e}"));
            fallback
        }
        Err(_) => {
            log("panic in native keyboard code");
            fallback
        }
    }
}

/// Run `f` on the keyboard behind `h` (0 = not created: `fallback`).
fn with<T>(h: jlong, fallback: T, f: impl FnOnce(&mut Handle) -> Result<T>) -> T {
    if h == 0 {
        return fallback;
    }
    // SAFETY: `h` came from `create` (Box::into_raw) and is only used from the
    // IME's main thread until `destroy`.
    let handle = unsafe { &mut *(h as *mut Handle) };
    guard(fallback, || {
        let out = f(handle);
        handle.flush_log();
        handle.save_grips();
        handle.save_words();
        out
    })
}

#[no_mangle]
pub extern "system" fn Java_io_github_timsalguz_rustkb_Native_create(
    mut env: JNIEnv,
    _: JClass,
    dict: JString,
    bigrams: JString,
    casing: JString,
    lemmas: JString,
    chooser: JString,
    settings: JString,
    log: JString,
    locale: JString,
    density: jfloat,
    sdk: jint,
) -> jlong {
    guard(0, || {
        let path: String = env.get_string(&dict)?.into();
        let bigrams: Option<String> = if bigrams.is_null() {
            None
        } else {
            Some(env.get_string(&bigrams)?.into())
        };
        let casing: Option<String> = if casing.is_null() {
            None
        } else {
            Some(env.get_string(&casing)?.into())
        };
        let lemmas = opt_string(&mut env, &lemmas)?;
        let chooser = opt_string(&mut env, &chooser)?;
        let settings: String = env.get_string(&settings)?.into();
        let log_path: Option<String> = if log.is_null() {
            None
        } else {
            Some(env.get_string(&log)?.into())
        };
        let locale = opt_string(&mut env, &locale)?.unwrap_or_default();
        first_settings(&settings, &locale);
        let mut engine = open_engine(&path, bigrams, casing, phone_config(kbcore::Latin::Qwerty))?;
        // The lemma vectors: without them the keyboard works as before.
        match lemmas.map(kbcore::Lemmas::open) {
            Some(Ok(l)) => {
                engine = engine.with_lemmas(l);
                // The chooser reads the lemma vectors: only with them.
                match chooser.map(kbcore::Chooser::open) {
                    Some(Ok(c)) => engine = engine.with_chooser(c),
                    Some(Err(e)) => crate::log(&format!("chooser: {e}")),
                    None => {}
                }
            }
            Some(Err(e)) => crate::log(&format!("lemmas: {e}")),
            None => {}
        }
        let grip_path = std::path::Path::new(&settings).with_file_name("grip.conf");
        let words_path = std::path::Path::new(&settings).with_file_name("user_words.txt");
        let mut ime = Ime::new(engine, density);
        ime.set_ui(i18n::ui_lang(&locale));
        ime.set_platform(sdk);
        if let Ok(text) = std::fs::read_to_string(&grip_path) {
            ime.load_grips(&text);
        }
        if let Ok(text) = std::fs::read_to_string(&words_path) {
            ime.load_user_words(&text);
        }
        let mut handle = Box::new(Handle {
            ime,
            grip_path,
            words_path,
            texts: Vec::new(),
            settings: SettingsFile::new(settings),
            log_path: log_path.map(Into::into),
        });
        handle.reload_settings();
        Ok(Box::into_raw(handle) as jlong)
    })
}

/// The clipboard changed: its text (null: none), whether the app marked it
/// sensitive (a password), when (the touches' clock). Flags as from touch.
#[no_mangle]
pub extern "system" fn Java_io_github_timsalguz_rustkb_Native_clip(
    mut env: JNIEnv,
    _: JClass,
    h: jlong,
    text: JString,
    sensitive: jboolean,
    time_ms: jlong,
) -> jint {
    with(h, 0, |k| {
        let text = opt_string(&mut env, &text)?;
        Ok(k.ime.clip(text.as_deref(), sensitive != 0, time_ms))
    })
}

/// No settings yet (the first start): the phone's language and English.
fn first_settings(path: &str, locale: &str) {
    let path = std::path::Path::new(path);
    if !path.exists() {
        let _ = Settings::for_locale(locale).save(path);
    }
}

/// The data packs to install, comma-separated (see [`PACKS`]).
#[no_mangle]
pub extern "system" fn Java_io_github_timsalguz_rustkb_Native_wantedPacks<'a>(
    env: JNIEnv<'a>,
    _: JClass,
    h: jlong,
) -> JString<'a> {
    let packs = with(h, String::new(), |k| Ok(k.ime.wanted_packs().join(",")));
    env.new_string(packs).unwrap_or_default()
}

/// A data pack, installed: its dictionary (null: the app has none), context
/// model and capitals list.
#[no_mangle]
pub extern "system" fn Java_io_github_timsalguz_rustkb_Native_addPack(
    mut env: JNIEnv,
    _: JClass,
    h: jlong,
    pack: JString,
    dict: JString,
    bigrams: JString,
    casing: JString,
) -> jint {
    with(h, 0, |k| {
        let pack: String = env.get_string(&pack)?.into();
        let Some(lang) = Lang::ALL.into_iter().find(|l| l.pack() == pack) else {
            return Ok(0);
        };
        let dict = opt_string(&mut env, &dict)?;
        let bigrams = opt_string(&mut env, &bigrams)?;
        let casing = opt_string(&mut env, &casing)?;
        let engine = match dict {
            Some(d) => match open_engine(&d, bigrams, casing, phone_config(lang.latin())) {
                Ok(e) => Some(e),
                Err(e) => {
                    log(&format!("pack {pack}: {e}"));
                    None
                }
            },
            None => None,
        };
        k.ime.add_pack(lang.pack(), engine);
        Ok(REDRAW)
    })
}

#[no_mangle]
pub extern "system" fn Java_io_github_timsalguz_rustkb_Native_destroy(
    _: JNIEnv,
    _: JClass,
    h: jlong,
) {
    if h != 0 {
        // SAFETY: `h` came from `create` and the shim never uses it after this.
        guard((), || {
            drop(unsafe { Box::from_raw(h as *mut Handle) });
            Ok(())
        });
    }
}

#[no_mangle]
pub extern "system" fn Java_io_github_timsalguz_rustkb_Native_measure(
    _: JNIEnv,
    _: JClass,
    h: jlong,
    width: jint,
) -> jint {
    with(h, 0, |k| Ok(k.ime.measure(width)))
}

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_io_github_timsalguz_rustkb_Native_startInput(
    mut env: JNIEnv,
    _: JClass,
    h: jlong,
    before: JString,
    after: JString,
    input_type: jint,
    ime_options: jint,
    hint_locales: JString,
    own_field: jboolean,
) -> jint {
    with(h, 0, |k| {
        let before: String = if before.is_null() {
            String::new()
        } else {
            env.get_string(&before)?.into()
        };
        let after: String = if after.is_null() {
            String::new()
        } else {
            env.get_string(&after)?.into()
        };
        k.ime.set_after(&after);
        const IME_FLAG_NO_PERSONALIZED_LEARNING: jint = 0x100_0000;
        k.ime
            .set_private(ime_options & IME_FLAG_NO_PERSONALIZED_LEARNING != 0);
        let hints = opt_string(&mut env, &hint_locales)?.unwrap_or_default();
        k.ime.set_hint_languages(&hints);
        k.reload_settings();
        // Debug log only in the app's own test field — never in other apps.
        k.ime.set_logging(own_field != 0);
        k.ime.set_own_field(own_field != 0);
        let flags = k.ime.start_input(&before, input_type);
        Ok(flags | k.packs_flag())
    })
}

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_io_github_timsalguz_rustkb_Native_selection(
    mut env: JNIEnv,
    _: JClass,
    h: jlong,
    sel_start: jint,
    sel_end: jint,
    cand_start: jint,
    cand_end: jint,
    before: JString,
    after: JString,
    selected: JString,
) -> jint {
    with(h, 0, |k| {
        let before: String = if before.is_null() {
            String::new()
        } else {
            env.get_string(&before)?.into()
        };
        let after: String = if after.is_null() {
            String::new()
        } else {
            env.get_string(&after)?.into()
        };
        let selected: Option<String> = if selected.is_null() {
            None
        } else {
            Some(env.get_string(&selected)?.into())
        };
        Ok(k.ime.selection(
            sel_start,
            sel_end,
            cand_start,
            cand_end,
            &before,
            &after,
            selected.as_deref(),
        ))
    })
}

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_io_github_timsalguz_rustkb_Native_touch(
    _: JNIEnv,
    _: JClass,
    h: jlong,
    action: jint,
    pointer: jint,
    x: jfloat,
    y: jfloat,
    time: jlong,
) -> jint {
    with(h, 0, |k| {
        if action == crate::ime::DOWN {
            k.reload_settings(); // a calibration may have been asked for
        }
        let flags = k.ime.touch(action, pointer, x, y, time);
        Ok(flags | if action == DOWN { k.packs_flag() } else { 0 })
    })
}

/// The settings file changed (the settings screen saved it): apply it now,
/// with the keyboard on screen — colors, height, languages, the strip.
#[no_mangle]
pub extern "system" fn Java_io_github_timsalguz_rustkb_Native_reloadSettings(
    _: JNIEnv,
    _: JClass,
    h: jlong,
) -> jint {
    with(h, 0, |k| {
        Ok(if k.reload_settings() {
            REDRAW | RELAYOUT | k.packs_flag()
        } else {
            0
        })
    })
}

#[no_mangle]
pub extern "system" fn Java_io_github_timsalguz_rustkb_Native_tilt(
    _: JNIEnv,
    _: JClass,
    h: jlong,
    x: jfloat,
    y: jfloat,
    z: jfloat,
) -> jint {
    with(h, 0, |k| Ok(k.ime.tilt(x, y, z)))
}

#[no_mangle]
pub extern "system" fn Java_io_github_timsalguz_rustkb_Native_timer(
    _: JNIEnv,
    _: JClass,
    h: jlong,
) -> jint {
    with(h, 0, |k| Ok(k.ime.timer()))
}

#[no_mangle]
pub extern "system" fn Java_io_github_timsalguz_rustkb_Native_drawOps(
    env: JNIEnv,
    _: JClass,
    h: jlong,
) -> jintArray {
    with(h, std::ptr::null_mut(), |k| {
        let (ops, texts) = k.ime.draw();
        k.texts = texts;
        let arr = env.new_int_array(ops.len() as jint)?;
        env.set_int_array_region(&arr, 0, &ops)?;
        Ok(arr.into_raw())
    })
}

#[no_mangle]
pub extern "system" fn Java_io_github_timsalguz_rustkb_Native_drawTexts(
    mut env: JNIEnv,
    _: JClass,
    h: jlong,
) -> jobjectArray {
    with(h, std::ptr::null_mut(), |k| {
        let arr =
            env.new_object_array(k.texts.len() as jint, "java/lang/String", JObject::null())?;
        for (i, s) in k.texts.iter().enumerate() {
            let js = env.new_string(s)?;
            env.set_object_array_element(&arr, i as jint, &js)?;
            env.delete_local_ref(js)?;
        }
        Ok(arr.into_raw())
    })
}

#[no_mangle]
pub extern "system" fn Java_io_github_timsalguz_rustkb_Native_takeOutput(
    env: JNIEnv,
    _: JClass,
    h: jlong,
) -> jstring {
    with(h, std::ptr::null_mut(), |k| {
        Ok(env.new_string(k.ime.take_output())?.into_raw())
    })
}

/// The settings screen's rows (see [`Settings::schema`]).
#[no_mangle]
pub extern "system" fn Java_io_github_timsalguz_rustkb_Native_settingsSchema(
    mut env: JNIEnv,
    _: JClass,
    path: JString,
    locale: JString,
) -> jobjectArray {
    guard(std::ptr::null_mut(), || {
        let path: String = env.get_string(&path)?.into();
        let locale = opt_string(&mut env, &locale)?.unwrap_or_default();
        first_settings(&path, &locale);
        let rows = Settings::load(std::path::Path::new(&path)).schema(i18n::ui_lang(&locale));
        let arr = env.new_object_array(rows.len() as jint, "java/lang/String", JObject::null())?;
        for (i, row) in rows.iter().enumerate() {
            let js = env.new_string(row)?;
            env.set_object_array_element(&arr, i as jint, &js)?;
            env.delete_local_ref(js)?;
        }
        Ok(arr.into_raw())
    })
}

/// The settings screen's own texts in the phone's language, `key\ttext`.
#[no_mangle]
pub extern "system" fn Java_io_github_timsalguz_rustkb_Native_screenTexts(
    mut env: JNIEnv,
    _: JClass,
    locale: JString,
) -> jobjectArray {
    guard(std::ptr::null_mut(), || {
        let locale = opt_string(&mut env, &locale)?.unwrap_or_default();
        let rows = i18n::screen_texts(i18n::ui_lang(&locale));
        let arr = env.new_object_array(rows.len() as jint, "java/lang/String", JObject::null())?;
        for (i, row) in rows.iter().enumerate() {
            let js = env.new_string(row)?;
            env.set_object_array_element(&arr, i as jint, &js)?;
            env.delete_local_ref(js)?;
        }
        Ok(arr.into_raw())
    })
}

/// The phone's light/dark mode and, on Android 12+, the wallpaper's colors
/// for the keyboard (two palettes of 8 ARGB colors, light then dark: null
/// without them). Returns the background color (for the navigation bar).
#[no_mangle]
pub extern "system" fn Java_io_github_timsalguz_rustkb_Native_systemColors(
    env: JNIEnv,
    _: JClass,
    h: jlong,
    night: jboolean,
    wallpaper: JIntArray,
) -> jint {
    with(h, 0, |k| {
        let colors: Option<Vec<i32>> = if wallpaper.is_null() {
            None
        } else {
            let n = env.get_array_length(&wallpaper)? as usize;
            let mut buf = vec![0i32; n];
            env.get_int_array_region(&wallpaper, 0, &mut buf)?;
            Some(buf)
        };
        k.ime.set_system_colors(night != 0, colors.as_deref());
        Ok(k.ime.background())
    })
}

#[no_mangle]
pub extern "system" fn Java_io_github_timsalguz_rustkb_Native_setSetting(
    mut env: JNIEnv,
    _: JClass,
    path: JString,
    key: JString,
    value: JString,
) {
    guard((), || {
        let path: String = env.get_string(&path)?.into();
        let key: String = env.get_string(&key)?.into();
        let value: String = env.get_string(&value)?.into();
        let path = std::path::Path::new(&path);
        let mut s = Settings::load(path);
        if s.set(&key, &value) {
            s.save(path)?;
        }
        Ok(())
    })
}
