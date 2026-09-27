package io.github.timsalguz.rustkb;

import android.annotation.SuppressLint;
import android.graphics.Canvas;
import android.graphics.Paint;
import android.graphics.RectF;
import android.view.MotionEvent;
import android.view.View;

/** Paints the draw list the Rust core returns and forwards touches to it. */
@SuppressLint("ViewConstructor")
final class KeyboardView extends View {
    // Draw-list opcodes (crates/android/src/ime.rs), 7 ints per op.
    private static final int OP_RECT = 1, OP_TEXT = 2, OP_ROTATE = 3, OP_RESTORE = 4;
    // Touch actions understood by the core.
    private static final int DOWN = 0, MOVE = 1, UP = 2, CANCEL = 3;

    private final RustKbService ime;
    private final Paint fill = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final Paint text = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final RectF rect = new RectF();

    KeyboardView(RustKbService ime) {
        super(ime);
        this.ime = ime;
        text.setTextAlign(Paint.Align.CENTER);
    }

    @Override
    protected void onMeasure(int widthSpec, int heightSpec) {
        int width = MeasureSpec.getSize(widthSpec);
        setMeasuredDimension(width, Native.measure(ime.handle(), width));
    }

    @Override
    protected void onDraw(Canvas canvas) {
        int[] ops = Native.drawOps(ime.handle());
        String[] texts = Native.drawTexts(ime.handle());
        if (ops == null || texts == null) return;
        for (int i = 0; i + 7 <= ops.length; i += 7) {
            if (ops[i] == OP_RECT) {
                fill.setColor(ops[i + 5]);
                rect.set(ops[i + 1], ops[i + 2], ops[i + 1] + ops[i + 3], ops[i + 2] + ops[i + 4]);
                canvas.drawRoundRect(rect, ops[i + 6], ops[i + 6], fill);
            } else if (ops[i] == OP_TEXT) {
                text.setTextSize(ops[i + 3]);
                text.setColor(ops[i + 4]);
                text.setFakeBoldText(ops[i + 6] != 0);
                canvas.drawText(texts[ops[i + 5]], ops[i + 1], ops[i + 2], text);
            } else if (ops[i] == OP_ROTATE) {
                // The arc layout's keys: turned about their centers until OP_RESTORE.
                canvas.save();
                canvas.rotate(ops[i + 3] / 1000f, ops[i + 1], ops[i + 2]);
            } else if (ops[i] == OP_RESTORE) {
                canvas.restore();
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
                    ime.respond(Native.touch(ime.handle(), MOVE, e.getPointerId(p), e.getX(p), e.getY(p), e.getEventTime()));
                }
                return true;
            default: return true;
        }
        int p = e.getActionIndex();
        ime.respond(Native.touch(ime.handle(), action, e.getPointerId(p), e.getX(p), e.getY(p), e.getEventTime()));
        return true;
    }
}
