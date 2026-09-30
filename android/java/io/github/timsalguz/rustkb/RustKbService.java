package io.github.timsalguz.rustkb;

import android.content.ClipData;
import android.content.ClipDescription;
import android.content.ClipboardManager;
import android.content.Context;
import android.content.res.Configuration;
import android.graphics.Color;
import android.os.Build;
import android.os.FileObserver;
import android.os.Handler;
import android.os.Looper;
import android.os.PersistableBundle;
import android.os.SystemClock;
import android.content.Intent;
import android.hardware.Sensor;
import android.hardware.SensorEvent;
import android.hardware.SensorEventListener;
import android.hardware.SensorManager;
import android.inputmethodservice.InputMethodService;
import android.util.Log;
import android.view.HapticFeedbackConstants;
import android.view.View;
import android.view.Window;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputMethodManager;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;

/**
 * Thin shim over the Rust core (libkbime.so): it forwards editor events,
 * applies the text operations the core returns and schedules its timers.
 * Every keyboard decision is made in Rust.
 */
public final class RustKbService extends InputMethodService implements SensorEventListener {
    // Response flags (crates/android/src/ime.rs); timer delay in the high 16 bits.
    private static final int REDRAW = 1, OUTPUT = 2, HAPTIC = 4, RELAYOUT = 8, PACKS = 16;

    private long handle;
    private KeyboardView view;
    private final Runnable timer = () -> respond(Native.timer(handle));
    /** Tilt, for telling the grip: only while the keyboard is up, at the UI rate. */
    private SensorManager sensors;
    private boolean listening;
    /**
     * The settings screen saves the file (write + rename): the keyboard takes
     * the new settings at once, even while it is on screen. Inotify, no polling.
     */
    private FileObserver settingsWatch;
    private final Handler main = new Handler(Looper.getMainLooper());
    /** Copied texts go to the core, which keeps them in memory only (never on disk). */
    private ClipboardManager clipboard;
    private final ClipboardManager.OnPrimaryClipChangedListener clipListener = this::clipChanged;

    @Override
    public void onCreate() {
        super.onCreate();
        try {
            // Debug log of the settings screen's test field, where `adb pull` reaches it:
            // /sdcard/Android/data/<package>/files/typing-log.jsonl
            File ext = getExternalFilesDir(null);
            String log = ext == null ? null : new File(ext, "typing-log.jsonl").getPath();
            // A ranked dictionary's priors lie beside it (read by the core).
            install("dict.bin");
            handle = Native.create(install("dict.fst"), install("bigrams.fst"), install("casing.fst"),
                    install("lemmas.bin"), install("chooser.bin"), install("parser.bin"), settingsPath(this), log,
                    locale(),
                    getResources().getDisplayMetrics().density, Build.VERSION.SDK_INT);
        } catch (Exception e) {
            Log.e("rustkb", "cannot open the dictionary", e);
        }
        String name = new File(settingsPath(this)).getName();
        settingsWatch = new FileObserver(getFilesDir().getPath(), FileObserver.MOVED_TO | FileObserver.CLOSE_WRITE) {
            @Override
            public void onEvent(int event, String path) {
                if (!name.equals(path)) return;
                main.post(() -> {
                    if (handle == 0) return;
                    respond(Native.reloadSettings(handle));
                    systemColors();
                });
            }
        };
        settingsWatch.startWatching();
        clipboard = (ClipboardManager) getSystemService(CLIPBOARD_SERVICE);
        if (clipboard != null) clipboard.addPrimaryClipChangedListener(clipListener);
    }

    /** The clipboard changed: its text to the core, unless the app marked it sensitive (a password). */
    private void clipChanged() {
        if (handle == 0 || clipboard == null) return;
        String text = null;
        boolean sensitive = false;
        try {
            ClipData clip = clipboard.getPrimaryClip();
            if (clip != null && clip.getItemCount() > 0) {
                CharSequence s = clip.getItemAt(0).coerceToText(this);
                text = s == null ? null : s.toString();
                ClipDescription d = clip.getDescription();
                PersistableBundle extras = d == null ? null : d.getExtras();
                sensitive = extras != null && extras.getBoolean("android.content.extra.IS_SENSITIVE", false);
            }
        } catch (RuntimeException e) {
            // Not the current keyboard (Android 10+ lets only it read the clipboard).
            return;
        }
        respond(Native.clip(handle, text, sensitive, SystemClock.uptimeMillis()));
    }

    @Override
    public void onDestroy() {
        if (settingsWatch != null) settingsWatch.stopWatching();
        if (clipboard != null) clipboard.removePrimaryClipChangedListener(clipListener);
        stopTilt();
        Native.destroy(handle);
        handle = 0;
        super.onDestroy();
    }

    @Override
    public View onCreateInputView() {
        view = new KeyboardView(this);
        return view;
    }

    @Override
    public void onStartInputView(EditorInfo info, boolean restarting) {
        super.onStartInputView(info, restarting);
        // Only the settings screen's own test field is logged.
        boolean own = getPackageName().equals(info.packageName);
        systemColors();
        respond(Native.startInput(handle, textBefore(), textAfter(), info.inputType, info.imeOptions,
                hintLocales(info), own));
        startTilt();
    }

    @Override
    public void onFinishInputView(boolean finishingInput) {
        stopTilt();
        super.onFinishInputView(finishingInput);
    }

    @Override
    public void onConfigurationChanged(Configuration config) {
        super.onConfigurationChanged(config);
        systemColors();
        if (view != null) view.invalidate();
    }

    /**
     * Tell the core the phone's dark mode and, on Android 12+, the wallpaper's
     * colors; paint the navigation bar like the keyboard.
     */
    private void systemColors() {
        boolean night = (getResources().getConfiguration().uiMode & Configuration.UI_MODE_NIGHT_MASK)
                == Configuration.UI_MODE_NIGHT_YES;
        int[] wallpaper = null;
        if (Build.VERSION.SDK_INT >= 31) {
            int[] ids = {
                // Light: background, key, special key, pressed, accent, text, dim text, text on accent.
                android.R.color.system_neutral2_100, android.R.color.system_neutral1_10,
                android.R.color.system_neutral2_200, android.R.color.system_neutral2_300,
                android.R.color.system_accent1_600, android.R.color.system_neutral1_900,
                android.R.color.system_neutral2_600, android.R.color.system_accent1_0,
                // Dark.
                android.R.color.system_neutral1_900, android.R.color.system_neutral2_700,
                android.R.color.system_neutral2_800, android.R.color.system_neutral2_600,
                android.R.color.system_accent1_200, android.R.color.system_neutral1_50,
                android.R.color.system_neutral2_400, android.R.color.system_accent1_800,
            };
            wallpaper = new int[ids.length];
            for (int i = 0; i < ids.length; i++) wallpaper[i] = getColor(ids[i]);
        }
        int bg = Native.systemColors(handle, night, wallpaper);
        Window w = getWindow() == null ? null : getWindow().getWindow();
        if (w == null) return;
        w.setNavigationBarColor(bg);
        View decor = w.getDecorView();
        double luminance = 0.2126 * Color.red(bg) + 0.7152 * Color.green(bg) + 0.0722 * Color.blue(bg);
        int flags = decor.getSystemUiVisibility();
        flags = luminance > 140 ? flags | View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR
                : flags & ~View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR;
        decor.setSystemUiVisibility(flags);
    }

    private void startTilt() {
        if (listening) return;
        if (sensors == null) sensors = (SensorManager) getSystemService(SENSOR_SERVICE);
        Sensor accel = sensors == null ? null : sensors.getDefaultSensor(Sensor.TYPE_ACCELEROMETER);
        listening = accel != null && sensors.registerListener(this, accel, SensorManager.SENSOR_DELAY_UI);
    }

    private void stopTilt() {
        if (listening) sensors.unregisterListener(this);
        listening = false;
    }

    @Override
    public void onSensorChanged(SensorEvent e) {
        respond(Native.tilt(handle, e.values[0], e.values[1], e.values[2]));
    }

    @Override
    public void onAccuracyChanged(Sensor sensor, int accuracy) {}

    private String textBefore() {
        InputConnection ic = getCurrentInputConnection();
        CharSequence before = ic == null ? null : ic.getTextBeforeCursor(48, 0);
        return before == null ? "" : before.toString();
    }

    /** The languages the app suggests for the field (Duolingo: the lesson's), "es,en". */
    private static String hintLocales(EditorInfo info) {
        if (Build.VERSION.SDK_INT < 24 || info.hintLocales == null) return "";
        StringBuilder out = new StringBuilder();
        for (int i = 0; i < info.hintLocales.size(); i++) {
            if (i > 0) out.append(',');
            out.append(info.hintLocales.get(i).getLanguage());
        }
        return out.toString();
    }

    private String textAfter() {
        InputConnection ic = getCurrentInputConnection();
        CharSequence after = ic == null ? null : ic.getTextAfterCursor(32, 0);
        return after == null ? "" : after.toString();
    }

    @Override
    public void onUpdateSelection(int oldSelStart, int oldSelEnd, int newSelStart, int newSelEnd,
                                  int candidatesStart, int candidatesEnd) {
        super.onUpdateSelection(oldSelStart, oldSelEnd, newSelStart, newSelEnd, candidatesStart, candidatesEnd);
        // A selected word may go into (or out of) the user's dictionary.
        String selected = null;
        InputConnection ic = getCurrentInputConnection();
        if (ic != null && newSelStart != newSelEnd) {
            CharSequence s = ic.getSelectedText(0);
            if (s != null && s.length() <= 64) selected = s.toString();
        }
        respond(Native.selection(handle, newSelStart, newSelEnd, candidatesStart, candidatesEnd, textBefore(),
                textAfter(), selected));
    }

    /** The phone's language ("de"): the first settings start with it. */
    static String locale() {
        return java.util.Locale.getDefault().getLanguage();
    }

    /** Install the language packs the settings ask for and hand them to the core. */
    private void installPacks() {
        String wanted = Native.wantedPacks(handle);
        if (wanted == null || wanted.isEmpty()) return;
        for (String pack : wanted.split(",")) {
            String dict = null, bigrams = null, casing = null;
            try {
                install(pack + "/dict.bin");
                dict = install(pack + "/dict.fst");
                if (dict != null) {
                    bigrams = install(pack + "/bigrams.fst");
                    casing = install(pack + "/casing.fst");
                }
            } catch (Exception e) {
                Log.e("rustkb", "cannot install " + pack, e);
                dict = null;
            }
            Native.addPack(handle, pack, dict, bigrams, casing);
        }
    }

    /** Shared with the settings screen; the core re-reads it when it changes. */
    static String settingsPath(Context context) {
        return new File(context.getFilesDir(), "settings.conf").getPath();
    }

    long handle() {
        return handle;
    }

    /** Act on a response from the core: text operations, haptics, redraw, timer. */
    void respond(int flags) {
        if ((flags & PACKS) != 0) installPacks();
        if ((flags & OUTPUT) != 0) apply(Native.takeOutput(handle));
        if (view == null) return;
        if ((flags & HAPTIC) != 0) view.performHapticFeedback(HapticFeedbackConstants.KEYBOARD_TAP);
        if ((flags & RELAYOUT) != 0) view.requestLayout();
        if ((flags & REDRAW) != 0) view.invalidate();
        int delay = flags >>> 16;
        if (delay > 0) {
            view.removeCallbacks(timer);
            view.postDelayed(timer, delay);
        }
    }

    /**
     * Ops are separated by NUL: C commit, Z composing, F finish, D delete, E enter,
     * K step over the next char, R replace the word around the cursor, P keyboard picker,
     * S settings.
     */
    private void apply(String ops) {
        InputConnection ic = getCurrentInputConnection();
        if (ic == null || ops == null || ops.isEmpty()) return;
        ic.beginBatchEdit();
        for (String op : ops.split("\0")) {
            if (op.isEmpty()) continue;
            String arg = op.substring(1);
            switch (op.charAt(0)) {
                case 'C': ic.commitText(arg, 1); break;
                case 'Z': ic.setComposingText(arg, 1); break;
                case 'F': ic.finishComposingText(); break;
                // Counts are in chars (code points): an emoji is one, not two.
                case 'D': ic.deleteSurroundingTextInCodePoints(Integer.parseInt(arg), 0); break;
                case 'E': if (!sendDefaultEditorAction(true)) ic.commitText("\n", 1); break;
                case 'K': ic.commitText("", 2); break;
                case 'R': {
                    // A word around the cursor, replaced: "before\tafter\ttext".
                    String[] r = arg.split("\t", 3);
                    ic.deleteSurroundingTextInCodePoints(Integer.parseInt(r[0]), Integer.parseInt(r[1]));
                    ic.commitText(r.length > 2 ? r[2] : "", 1);
                    break;
                }
                case 'P':
                    ((InputMethodManager) getSystemService(INPUT_METHOD_SERVICE)).showInputMethodPicker();
                    break;
                case 'S':
                    startActivity(new Intent(this, SettingsActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
                    break;
                default: break;
            }
        }
        ic.endBatchEdit();
    }

    /**
     * Data files ship as APK assets; the core mmaps them from plain files,
     * copied once per app update. Returns null if the asset isn't packaged.
     */
    private String install(String asset) throws Exception {
        // A pack's files ("de/dict.fst") live flat: de-dict-<stamp>.fst; a
        // file keeps its ending (dict.bin beside dict.fst: dict-<stamp>.bin).
        int dot = asset.lastIndexOf('.');
        String base = asset.substring(0, dot).replace('/', '-');
        String ext = asset.substring(dot);
        long stamp = getPackageManager().getPackageInfo(getPackageName(), 0).lastUpdateTime;
        File file = new File(getFilesDir(), base + "-" + stamp + ext);
        if (!file.exists()) {
            File[] old = getFilesDir().listFiles();
            if (old != null) {
                for (File f : old) {
                    String n = f.getName();
                    if (n.startsWith(base + "-") && n.endsWith(ext)) f.delete();
                }
            }
            File tmp = new File(getFilesDir(), base + ext + ".tmp");
            try (InputStream in = getAssets().open(asset); OutputStream out = new FileOutputStream(tmp)) {
                byte[] buf = new byte[1 << 16];
                for (int n; (n = in.read(buf)) > 0; ) out.write(buf, 0, n);
            } catch (java.io.FileNotFoundException e) {
                return null;
            }
            if (!tmp.renameTo(file)) throw new IOException("cannot install " + file);
        }
        return file.getPath();
    }
}
