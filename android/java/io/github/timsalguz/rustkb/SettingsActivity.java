package io.github.timsalguz.rustkb;

import android.app.Activity;
import android.content.Intent;
import android.content.SharedPreferences;
import android.net.Uri;
import android.os.Bundle;
import android.provider.Settings;
import android.view.View;
import android.view.inputmethod.InputMethodManager;
import android.widget.Button;
import android.widget.CheckBox;
import android.text.InputType;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.RadioButton;
import android.widget.RadioGroup;
import android.widget.ScrollView;
import android.widget.Switch;
import android.widget.TextView;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * Settings screen. The rows come from the Rust core (Settings::schema), so a
 * new setting needs no Java: "key \t bool|choice \t label \t value \t options".
 */
public final class SettingsActivity extends Activity {
    private static final String REPO = "https://github.com/TimSalguz/rust-android-kb";
    /** The screen's own texts in the phone's language (crates/android/src/i18n.rs). */
    private final Map<String, String> texts = new HashMap<>();

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        String path = RustKbService.settingsPath(this);
        String[] own = Native.screenTexts(RustKbService.locale());
        for (String row : own == null ? new String[0] : own) {
            String[] kv = row.split("\t", 2);
            if (kv.length == 2) texts.put(kv[0], kv[1]);
        }
        int pad = (int) (16 * getResources().getDisplayMetrics().density);
        LinearLayout list = new LinearLayout(this);
        list.setOrientation(LinearLayout.VERTICAL);
        list.setPadding(pad, pad, pad, pad);

        // The guide: shown on the first start, then behind a button.
        SharedPreferences prefs = getSharedPreferences("screen", MODE_PRIVATE);
        TextView guide = new TextView(this);
        guide.setText(text("ui.guide"));
        guide.setPadding(0, 0, 0, pad / 2);
        guide.setTextSize(15);
        Button seen = button(text("ui.guide_ok"), null);
        Button again = button(text("ui.guide_button"), null);
        boolean first = !prefs.getBoolean("guide_seen", false);
        guide.setVisibility(first ? View.VISIBLE : View.GONE);
        seen.setVisibility(first ? View.VISIBLE : View.GONE);
        again.setVisibility(first ? View.GONE : View.VISIBLE);
        seen.setOnClickListener(v -> {
            prefs.edit().putBoolean("guide_seen", true).apply();
            guide.setVisibility(View.GONE);
            seen.setVisibility(View.GONE);
            again.setVisibility(View.VISIBLE);
        });
        again.setOnClickListener(v -> {
            guide.setVisibility(View.VISIBLE);
            seen.setVisibility(View.VISIBLE);
            again.setVisibility(View.GONE);
        });
        list.addView(guide);
        list.addView(seen);

        list.addView(button(text("ui.enable"),
                v -> startActivity(new Intent(Settings.ACTION_INPUT_METHOD_SETTINGS))));
        list.addView(button(text("ui.pick"),
                v -> ((InputMethodManager) getSystemService(INPUT_METHOD_SERVICE)).showInputMethodPicker()));
        list.addView(again);

        EditText test = new EditText(this);
        test.setHint(text("ui.try"));
        test.setInputType(InputType.TYPE_CLASS_TEXT
                | InputType.TYPE_TEXT_FLAG_CAP_SENTENCES
                | InputType.TYPE_TEXT_FLAG_MULTI_LINE);

        String[] rows = Native.settingsSchema(path, RustKbService.locale());
        for (String row : rows == null ? new String[0] : rows) {
            String[] f = row.split("\t", -1);
            String key = f[0];
            if (f[1].equals("actionpair")) {
                // Two buttons in a row: calibrate a grip | undo it.
                String[] second = f[4].split(":", 3);
                LinearLayout pair = new LinearLayout(this);
                pair.setOrientation(LinearLayout.HORIZONTAL);
                Button main = button(f[2], v -> {
                    Native.setSetting(path, key, String.valueOf(System.currentTimeMillis()));
                    test.clearFocus();
                    test.requestFocus();
                    ((InputMethodManager) getSystemService(INPUT_METHOD_SERVICE))
                            .showSoftInput(test, InputMethodManager.SHOW_IMPLICIT);
                });
                Button undo = button(second[1], v -> Native.setSetting(path, second[0],
                        String.valueOf(System.currentTimeMillis())));
                pair.addView(main, new LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 3f));
                pair.addView(undo, new LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f));
                list.addView(pair);
            } else if (f[1].equals("action")) {
                // A button: the keyboard acts on the new stamp; the calibration
                // runs in the test field below, with Rust KB as the keyboard.
                list.addView(button(f[2], v -> {
                    Native.setSetting(path, key, String.valueOf(System.currentTimeMillis()));
                    if (key.equals("calibrate")) {
                        test.clearFocus();
                        test.requestFocus();
                        ((InputMethodManager) getSystemService(INPUT_METHOD_SERVICE))
                                .showSoftInput(test, InputMethodManager.SHOW_IMPLICIT);
                    }
                }));
            } else if (f[1].equals("order")) {
                // Several of the options, in an order: tick them, move them up/down.
                TextView label = new TextView(this);
                label.setText(f[2]);
                label.setPadding(0, pad, 0, 0);
                list.addView(label);
                LinearLayout box = new LinearLayout(this);
                box.setOrientation(LinearLayout.VERTICAL);
                list.addView(box);
                List<String> order = new ArrayList<>(Arrays.asList(f[3].split(",")));
                showOrder(box, path, key, f[4].split("\\|"), order);
            } else if (f[1].equals("bool")) {
                Switch toggle = new Switch(this);
                toggle.setText(f[2]);
                toggle.setChecked(f[3].equals("1"));
                toggle.setPadding(0, pad / 2, 0, pad / 2);
                toggle.setOnCheckedChangeListener((b, on) -> Native.setSetting(path, key, on ? "1" : "0"));
                list.addView(toggle);
            } else {
                TextView label = new TextView(this);
                label.setText(f[2]);
                label.setPadding(0, pad, 0, 0);
                list.addView(label);
                RadioGroup group = new RadioGroup(this);
                group.setOrientation(RadioGroup.VERTICAL);
                for (String option : f[4].split("\\|")) {
                    String[] kv = option.split(":", 2);
                    RadioButton choice = new RadioButton(this);
                    choice.setId(View.generateViewId());
                    choice.setText(kv[1]);
                    group.addView(choice);
                    if (kv[0].equals(f[3])) group.check(choice.getId());
                    choice.setOnClickListener(v -> Native.setSetting(path, key, kv[0]));
                }
                list.addView(group);
            }
        }

        list.addView(test);
        list.addView(button(text("ui.github"),
                v -> startActivity(new Intent(Intent.ACTION_VIEW, Uri.parse(REPO)))));
        list.addView(button(text("ui.licenses"), v -> showLicenses(pad)));

        ScrollView scroll = new ScrollView(this);
        scroll.addView(list);
        setContentView(scroll);
    }

    /** The ticked options in their order (with ↑ ↓), then the others. */
    private void showOrder(LinearLayout box, String path, String key, String[] options, List<String> order) {
        box.removeAllViews();
        List<String[]> rows = new ArrayList<>();
        for (String code : order) {
            for (String o : options) if (o.startsWith(code + ":")) rows.add(o.split(":", 2));
        }
        for (String o : options) {
            String[] kv = o.split(":", 2);
            if (!order.contains(kv[0])) rows.add(kv);
        }
        for (String[] kv : rows) {
            String code = kv[0];
            int at = order.indexOf(code);
            LinearLayout row = new LinearLayout(this);
            row.setOrientation(LinearLayout.HORIZONTAL);
            CheckBox tick = new CheckBox(this);
            tick.setText(kv[1]);
            tick.setChecked(at >= 0);
            row.addView(tick, new LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f));
            tick.setOnCheckedChangeListener((b, on) -> {
                if (on && !order.contains(code)) order.add(code);
                if (!on && order.size() > 1) order.remove(code);
                Native.setSetting(path, key, String.join(",", order));
                box.post(() -> showOrder(box, path, key, options, order));
            });
            if (at >= 0 && order.size() > 1) {
                Button up = small("↑", at > 0, v -> {
                    order.remove(code);
                    order.add(at - 1, code);
                    Native.setSetting(path, key, String.join(",", order));
                    showOrder(box, path, key, options, order);
                });
                Button down = small("↓", at < order.size() - 1, v -> {
                    order.remove(code);
                    order.add(at + 1, code);
                    Native.setSetting(path, key, String.join(",", order));
                    showOrder(box, path, key, options, order);
                });
                row.addView(up);
                row.addView(down);
            }
            box.addView(row);
        }
    }

    private Button small(String text, boolean enabled, View.OnClickListener onClick) {
        Button b = button(text, onClick);
        b.setEnabled(enabled);
        b.setMinWidth(0);
        b.setMinimumWidth(0);
        return b;
    }

    /** The licenses and attributions of the code and the packaged data (assets/NOTICE.txt). */
    private void showLicenses(int pad) {
        StringBuilder all = new StringBuilder();
        try (java.io.InputStream in = getAssets().open("NOTICE.txt")) {
            byte[] buf = new byte[1 << 14];
            java.io.ByteArrayOutputStream out = new java.io.ByteArrayOutputStream();
            for (int n; (n = in.read(buf)) > 0; ) out.write(buf, 0, n);
            all.append(out.toString("UTF-8"));
        } catch (java.io.IOException e) {
            all.append(REPO);
        }
        TextView body = new TextView(this);
        body.setText(all);
        body.setTextIsSelectable(true);
        body.setPadding(pad, pad, pad, pad);
        ScrollView scroll = new ScrollView(this);
        scroll.addView(body);
        new android.app.AlertDialog.Builder(this)
                .setTitle(text("ui.licenses"))
                .setView(scroll)
                .setPositiveButton(android.R.string.ok, null)
                .show();
    }

    private String text(String key) {
        String t = texts.get(key);
        return t == null ? key : t;
    }

    private Button button(String text, View.OnClickListener onClick) {
        Button b = new Button(this);
        b.setText(text);
        b.setOnClickListener(onClick);
        return b;
    }
}
