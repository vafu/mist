package io.github.vafu.mist.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/**
 * Minimal icon set drawn with vectors.
 *
 * The project does not depend on material-icons, and these five glyphs are
 * simple enough that drawing them keeps the dependency surface small.
 */
@Composable
internal fun PlusIcon(modifier: Modifier = Modifier, size: Dp = 20.dp, tint: Color = Color.Unspecified) {
    Glyph(size, modifier, tint) { color, stroke ->
        drawLine(color, Offset(w * 0.5f, h * 0.2f), Offset(w * 0.5f, h * 0.8f), stroke, StrokeCap.Round)
        drawLine(color, Offset(w * 0.2f, h * 0.5f), Offset(w * 0.8f, h * 0.5f), stroke, StrokeCap.Round)
    }
}

@Composable
internal fun TrashIcon(modifier: Modifier = Modifier, size: Dp = 20.dp, tint: Color = Color.Unspecified) {
    Glyph(size, modifier, tint) { color, stroke ->
        drawLine(color, Offset(w * 0.18f, h * 0.28f), Offset(w * 0.82f, h * 0.28f), stroke, StrokeCap.Round)
        drawLine(color, Offset(w * 0.4f, h * 0.2f), Offset(w * 0.6f, h * 0.2f), stroke, StrokeCap.Round)
        val body = Path().apply {
            moveTo(w * 0.28f, h * 0.28f)
            lineTo(w * 0.33f, h * 0.82f)
            lineTo(w * 0.67f, h * 0.82f)
            lineTo(w * 0.72f, h * 0.28f)
        }
        drawPath(body, color, style = Stroke(stroke, cap = StrokeCap.Round, join = StrokeJoin.Round))
        drawLine(color, Offset(w * 0.44f, h * 0.42f), Offset(w * 0.45f, h * 0.70f), stroke * 0.7f, StrokeCap.Round)
        drawLine(color, Offset(w * 0.56f, h * 0.42f), Offset(w * 0.55f, h * 0.70f), stroke * 0.7f, StrokeCap.Round)
    }
}

@Composable
internal fun ClipboardIcon(modifier: Modifier = Modifier, size: Dp = 20.dp, tint: Color = Color.Unspecified) {
    Glyph(size, modifier, tint) { color, stroke ->
        val board = Path().apply {
            moveTo(w * 0.25f, h * 0.24f)
            lineTo(w * 0.75f, h * 0.24f)
            lineTo(w * 0.75f, h * 0.85f)
            lineTo(w * 0.25f, h * 0.85f)
            close()
        }
        drawPath(board, color, style = Stroke(stroke, join = StrokeJoin.Round))
        drawLine(color, Offset(w * 0.4f, h * 0.18f), Offset(w * 0.6f, h * 0.18f), stroke, StrokeCap.Round)
        drawLine(color, Offset(w * 0.36f, h * 0.5f), Offset(w * 0.64f, h * 0.5f), stroke * 0.8f, StrokeCap.Round)
        drawLine(color, Offset(w * 0.36f, h * 0.66f), Offset(w * 0.58f, h * 0.66f), stroke * 0.8f, StrokeCap.Round)
    }
}

@Composable
internal fun CheckIcon(modifier: Modifier = Modifier, size: Dp = 20.dp, tint: Color = Color.Unspecified) {
    Glyph(size, modifier, tint) { color, stroke ->
        drawLine(color, Offset(w * 0.2f, h * 0.52f), Offset(w * 0.43f, h * 0.75f), stroke, StrokeCap.Round)
        drawLine(color, Offset(w * 0.43f, h * 0.75f), Offset(w * 0.82f, h * 0.27f), stroke, StrokeCap.Round)
    }
}

@Composable
internal fun ShareIcon(modifier: Modifier = Modifier, size: Dp = 20.dp, tint: Color = Color.Unspecified) {
    Glyph(size, modifier, tint) { color, stroke ->
        drawCircle(color, radius = w * 0.09f, center = Offset(w * 0.72f, h * 0.22f), style = Stroke(stroke))
        drawCircle(color, radius = w * 0.09f, center = Offset(w * 0.28f, h * 0.5f), style = Stroke(stroke))
        drawCircle(color, radius = w * 0.09f, center = Offset(w * 0.72f, h * 0.78f), style = Stroke(stroke))
        drawLine(color, Offset(w * 0.36f, h * 0.46f), Offset(w * 0.64f, h * 0.27f), stroke * 0.8f, StrokeCap.Round)
        drawLine(color, Offset(w * 0.36f, h * 0.54f), Offset(w * 0.64f, h * 0.73f), stroke * 0.8f, StrokeCap.Round)
    }
}

@Composable
internal fun CameraIcon(modifier: Modifier = Modifier, size: Dp = 20.dp, tint: Color = Color.Unspecified) {
    Glyph(size, modifier, tint) { color, stroke ->
        val body = Path().apply {
            moveTo(w * 0.15f, h * 0.32f)
            lineTo(w * 0.32f, h * 0.32f)
            lineTo(w * 0.38f, h * 0.22f)
            lineTo(w * 0.62f, h * 0.22f)
            lineTo(w * 0.68f, h * 0.32f)
            lineTo(w * 0.85f, h * 0.32f)
            lineTo(w * 0.85f, h * 0.8f)
            lineTo(w * 0.15f, h * 0.8f)
            close()
        }
        drawPath(body, color, style = Stroke(stroke, join = StrokeJoin.Round))
        drawCircle(color, radius = w * 0.14f, center = Offset(w * 0.5f, h * 0.55f), style = Stroke(stroke))
    }
}

@Composable
private fun Glyph(
    size: Dp,
    modifier: Modifier,
    tint: Color,
    draw: DrawScope.(Color, Float) -> Unit,
) {
    val fallback = androidx.compose.material3.LocalContentColor.current
    Canvas(modifier = modifier.size(size)) {
        draw(if (tint == Color.Unspecified) fallback else tint, this.size.width * 0.09f)
    }
}

private val DrawScope.w: Float get() = size.width
private val DrawScope.h: Float get() = size.height
