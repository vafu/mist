package io.github.vafu.mist.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

private const val IconSize = 1024f

private val Background = Brush.verticalGradient(
    listOf(Color(0xFF35302A), Color(0xFF181716)),
)

private val Ink = Brush.verticalGradient(
    listOf(Color(0xFFF3E6C9), Color(0xFFC0A675)),
    startY = 280f,
    endY = 800f,
)

private val Structure = Color(0xFFD8C39A)

/**
 * The Mist launcher mark, using the exact geometry of the adaptive icon.
 *
 * Rendered over the icon's dark background so it stays legible in both color
 * schemes; pass [tint] to draw the mark alone in a single color.
 */
@Composable
internal fun AppMark(
    modifier: Modifier = Modifier,
    size: Dp = 32.dp,
    tint: Color? = null,
) {
    Canvas(modifier = modifier.size(size)) {
        val scale = this.size.width / IconSize
        if (tint == null) {
            drawRoundRect(
                brush = Background,
                cornerRadius = CornerRadius(this.size.width * 0.22f),
            )
        }
        scale(scale, scale, pivot = Offset.Zero) {
            drawMark(tint)
        }
    }
}

private fun DrawScope.drawMark(tint: Color?) {
    val ink = tint?.let { Brush.linearGradient(listOf(it, it)) } ?: Ink
    val structure = tint ?: Structure

    fun line(x1: Float, y1: Float, x2: Float, y2: Float, width: Float) {
        drawLine(
            brush = ink,
            start = Offset(x1, y1),
            end = Offset(x2, y2),
            strokeWidth = width,
            cap = StrokeCap.Round,
        )
    }

    // Cable: gentle anchors, tall towers, deep centre sag.
    drawPath(
        path = Path().apply {
            moveTo(120f, 706f)
            cubicTo(178f, 640f, 240f, 470f, 300f, 330f)
            cubicTo(380f, 640f, 460f, 690f, 512f, 690f)
            cubicTo(564f, 690f, 644f, 640f, 724f, 330f)
            cubicTo(784f, 470f, 846f, 640f, 904f, 706f)
        },
        brush = ink,
        style = Stroke(26f, cap = StrokeCap.Round, join = StrokeJoin.Round),
    )

    // Hangers.
    line(392f, 566f, 392f, 692f, 13f)
    line(452f, 640f, 452f, 692f, 13f)
    line(572f, 640f, 572f, 692f, 13f)
    line(632f, 566f, 632f, 692f, 13f)

    // Towers.
    line(300f, 306f, 300f, 800f, 40f)
    line(724f, 306f, 724f, 800f, 40f)
    line(266f, 306f, 334f, 306f, 28f)
    line(690f, 306f, 758f, 306f, 28f)

    // Deck and approach ramps.
    drawLine(
        color = structure,
        start = Offset(120f, 706f),
        end = Offset(904f, 706f),
        strokeWidth = 38f,
        cap = StrokeCap.Round,
    )
    drawLine(
        color = structure,
        start = Offset(120f, 706f),
        end = Offset(66f, 796f),
        strokeWidth = 26f,
        cap = StrokeCap.Round,
    )
    drawLine(
        color = structure,
        start = Offset(904f, 706f),
        end = Offset(958f, 796f),
        strokeWidth = 26f,
        cap = StrokeCap.Round,
    )
}
