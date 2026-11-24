package io.github.timsalguz.rustkb;

import android.annotation.SuppressLint;
import android.graphics.Canvas;
import android.graphics.Paint;
import android.graphics.RectF;
import android.os.Build;
import android.view.MotionEvent;
import android.view.View;
import android.view.WindowInsets;

/** Paints the latest immutable frame and forwards touches to the native worker. */
@SuppressLint("ViewConstructor")
final class KeyboardView extends View {
    // Draw-list opcodes (crates/android/src/ime.rs), 7 ints per op.
    private static final int OP_RECT = 1, OP_TEXT = 2, OP_ROTATE = 3, OP_RESTORE = 4, OP_ICON = 5;
    private static final int OP_CLIP = 6, OP_TEXT_LEFT = 7;
    private static final int ICON_SMILE = 1, ICON_SEARCH = 2, ICON_CLIPBOARD = 3;
    // Touch actions understood by the core.
    private static final int DOWN = 0, MOVE = 1, UP = 2, CANCEL = 3;

    private final RustKbService ime;
    private final Paint fill = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final Paint text = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final Paint line = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final RectF rect = new RectF();
    /**
     * The system's navigation bar over the bottom of the keyboard's window
     * (Android 15 draws an IME's window behind it): the keys stay above it,
     * the keyboard's background goes on under it.
     */
    private int navBottom;
    private volatile int[] drawOps = new int[0];
    private volatile String[] drawTexts = new String[0];
    private int frameWidth;
    private int frameHeight;

    void setDrawList(int[] ops, String[] texts) {
        drawTexts = texts;
        drawOps = ops;
        if (ops.length >= 7 && ops[0] == OP_RECT) {
            frameWidth = ops[3];
            frameHeight = Math.max(0, ops[4] - 1);
        }
    }

    KeyboardView(RustKbService ime) {
        super(ime);
        this.ime = ime;
        text.setTextAlign(Paint.Align.CENTER);
        line.setStyle(Paint.Style.STROKE);
        line.setStrokeCap(Paint.Cap.ROUND);
        setOnApplyWindowInsetsListener((v, insets) -> {
            int bottom;
            if (Build.VERSION.SDK_INT >= 30) {
                bottom = Math.max(
                        insets.getInsets(WindowInsets.Type.navigationBars()).bottom,
                        insets.getInsetsIgnoringVisibility(WindowInsets.Type.navigationBars()).bottom);
            } else {
                bottom = Math.max(insets.getSystemWindowInsetBottom(), insets.getStableInsetBottom());
            }
            if (bottom != navBottom) {
                navBottom = bottom;
                requestLayout();
            }
            return insets;
        });
    }

    @Override
    protected void onMeasure(int widthSpec, int heightSpec) {
        int width = MeasureSpec.getSize(widthSpec);
        if (width != frameWidth || frameHeight == 0) {
            frameHeight = ime.measure(width);
            frameWidth = width;
        }
        setMeasuredDimension(width, frameHeight + navBottom);
    }

    @Override
    protected void onDraw(Canvas canvas) {
        int[] ops = drawOps;
        String[] texts = drawTexts;
        // The background (the list's first rectangle) under the navigation bar too.
        if (ops.length >= 7 && ops[0] == OP_RECT) canvas.drawColor(ops[5]);
        for (int i = 0; i + 7 <= ops.length; i += 7) {
            if (ops[i] == OP_RECT) {
                fill.setColor(ops[i + 5]);
                rect.set(ops[i + 1], ops[i + 2], ops[i + 1] + ops[i + 3], ops[i + 2] + ops[i + 4]);
                canvas.drawRoundRect(rect, ops[i + 6], ops[i + 6], fill);
            } else if (ops[i] == OP_TEXT || ops[i] == OP_TEXT_LEFT) {
                text.setTextAlign(ops[i] == OP_TEXT ? Paint.Align.CENTER : Paint.Align.LEFT);
                text.setTextSize(ops[i + 3]);
                text.setColor(ops[i + 4]);
                text.setFakeBoldText(ops[i + 6] != 0);
                canvas.drawText(texts[ops[i + 5]], ops[i + 1], ops[i + 2], text);
            } else if (ops[i] == OP_CLIP) {
                canvas.save();
                canvas.clipRect(ops[i + 1], ops[i + 2],
                        ops[i + 1] + ops[i + 3], ops[i + 2] + ops[i + 4]);
            } else if (ops[i] == OP_ROTATE) {
                // The arc layout's keys: turned about their centers until OP_RESTORE.
                canvas.save();
                canvas.rotate(ops[i + 3] / 1000f, ops[i + 1], ops[i + 2]);
            } else if (ops[i] == OP_RESTORE) {
                canvas.restore();
            } else if (ops[i] == OP_ICON && ops[i + 5] == ICON_SMILE) {
                // A smile in one color: a ring, two eyes, a mouth.
                float cx = ops[i + 1], cy = ops[i + 2], r = ops[i + 3] / 2f;
                line.setColor(ops[i + 4]);
                line.setStrokeWidth(Math.max(1f, r * 0.16f));
                fill.setColor(ops[i + 4]);
                canvas.drawCircle(cx, cy, r, line);
                canvas.drawCircle(cx - r * 0.36f, cy - r * 0.22f, r * 0.12f, fill);
                canvas.drawCircle(cx + r * 0.36f, cy - r * 0.22f, r * 0.12f, fill);
                rect.set(cx - r * 0.5f, cy - r * 0.5f, cx + r * 0.5f, cy + r * 0.5f);
                canvas.drawArc(rect, 25, 130, false, line);
            } else if (ops[i] == OP_ICON && ops[i + 5] == ICON_SEARCH) {
                // A lens: a ring up and left, its handle down to the right.
                float cx = ops[i + 1], cy = ops[i + 2], r = ops[i + 3] / 2f;
                line.setColor(ops[i + 4]);
                line.setStrokeWidth(Math.max(1f, r * 0.2f));
                float lx = cx - r * 0.15f, ly = cy - r * 0.15f, lr = r * 0.6f;
                canvas.drawCircle(lx, ly, lr, line);
                float d = lr * 0.7071f;
                canvas.drawLine(lx + d, ly + d, cx + r * 0.85f, cy + r * 0.85f, line);
            } else if (ops[i] == OP_ICON && ops[i + 5] == ICON_CLIPBOARD) {
                float cx = ops[i + 1], cy = ops[i + 2], r = ops[i + 3] / 2f;
                line.setColor(ops[i + 4]);
                line.setStrokeWidth(Math.max(1f, r * 0.16f));
                rect.set(cx - r * 0.68f, cy - r * 0.7f, cx + r * 0.68f, cy + r * 0.9f);
                canvas.drawRoundRect(rect, r * 0.12f, r * 0.12f, line);
                rect.set(cx - r * 0.34f, cy - r * 0.95f, cx + r * 0.34f, cy - r * 0.48f);
                canvas.drawRoundRect(rect, r * 0.12f, r * 0.12f, line);
                canvas.drawLine(cx - r * 0.35f, cy - r * 0.1f, cx + r * 0.35f, cy - r * 0.1f, line);
                canvas.drawLine(cx - r * 0.35f, cy + r * 0.3f, cx + r * 0.35f, cy + r * 0.3f, line);
            }
        }
    }

    @SuppressLint("ClickableViewAccessibility")
    @Override
    public boolean onTouchEvent(MotionEvent e) {
        int action;
        switch (e.getActionMasked()) {
            case MotionEvent.ACTION_DOWN:
            case MotionEvent.ACTION_POINTER_DOWN: action = DOWN; break;
            case MotionEvent.ACTION_UP:
            case MotionEvent.ACTION_POINTER_UP: action = UP; break;
            case MotionEvent.ACTION_CANCEL: action = CANCEL; break;
            case MotionEvent.ACTION_MOVE:
                for (int p = 0; p < e.getPointerCount(); p++) {
                    ime.touch(MOVE, e.getPointerId(p), e.getX(p), e.getY(p), e.getEventTime());
                }
                return true;
            default: return true;
        }
        int p = e.getActionIndex();
        ime.touch(action, e.getPointerId(p), e.getX(p), e.getY(p), e.getEventTime());
        return true;
    }
}
