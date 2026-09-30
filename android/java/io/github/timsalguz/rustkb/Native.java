package io.github.timsalguz.rustkb;

/** JNI surface of libkbime.so (crates/android/src/lib.rs). */
final class Native {
    static {
        System.loadLibrary("kbime");
    }

    private Native() {}

    static native long create(String dictPath, String bigramsPath, String casingPath, String lemmasPath,
            String chooserPath, String parserPath, String settingsPath, String logPath, String locale, float density,
            int sdk);
    /** Language packs to install ("de,fr"), then hand over with addPack (dictPath null: not in the app). */
    static native String wantedPacks(long h);
    static native int addPack(long h, String pack, String dictPath, String bigramsPath, String casingPath);
    static native void destroy(long h);
    static native int measure(long h, int widthPx);
    static native int startInput(long h, String textBefore, String textAfter, int inputType, int imeOptions,
            String hintLocales, boolean ownField);
    static native int selection(long h, int selStart, int selEnd, int candStart, int candEnd, String textBefore,
            String textAfter, String selectedText);
    static native int touch(long h, int action, int pointer, float x, float y, long timeMs);
    static native int timer(long h);
    /** The settings file changed: re-read it (flags as from touch). */
    static native int reloadSettings(long h);
    static native int tilt(long h, float x, float y, float z);
    /** The clipboard changed: its text (null: none), sensitive (a password), when (uptime ms). */
    static native int clip(long h, String text, boolean sensitive, long timeMs);
    static native int[] drawOps(long h);
    static native String[] drawTexts(long h);
    static native String takeOutput(long h);

    // Settings screen (no keyboard instance needed).
    static native String[] settingsSchema(String settingsPath, String locale);
    /** The settings screen's own texts in the phone's language: "key\ttext". */
    static native String[] screenTexts(String locale);
    /** Dark mode and the wallpaper palettes (8 light + 8 dark ARGB, or null); returns the background. */
    static native int systemColors(long h, boolean night, int[] wallpaper);
    static native void setSetting(String settingsPath, String key, String value);
}
