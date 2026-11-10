package io.github.vafu.mist.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

private val LightColors = lightColorScheme(
    primary = Color(0xFF6F5B3A),
    onPrimary = Color(0xFFFFFFFF),
    primaryContainer = Color(0xFFF3E3C5),
    onPrimaryContainer = Color(0xFF251A00),
    secondary = Color(0xFF635E50),
    onSecondary = Color(0xFFFFFFFF),
    secondaryContainer = Color(0xFFEAE2D2),
    onSecondaryContainer = Color(0xFF1F1B11),
    tertiary = Color(0xFF3F6355),
    onTertiary = Color(0xFFFFFFFF),
    tertiaryContainer = Color(0xFFC1EAD7),
    onTertiaryContainer = Color(0xFF002117),
    surface = Color(0xFFFDF8F3),
    onSurface = Color(0xFF1D1B16),
    surfaceVariant = Color(0xFFE8E1D4),
    onSurfaceVariant = Color(0xFF4A463A),
    surfaceContainerLowest = Color(0xFFFFFFFF),
    surfaceContainerLow = Color(0xFFF7F2EC),
    surfaceContainer = Color(0xFFF1ECE5),
    surfaceContainerHigh = Color(0xFFECE6DF),
    surfaceContainerHighest = Color(0xFFE6E0D8),
    outline = Color(0xFF7B7666),
    outlineVariant = Color(0xFFCCC6B8),
    error = Color(0xFFBA1A1A),
    onError = Color(0xFFFFFFFF),
    errorContainer = Color(0xFFFFDAD6),
    onErrorContainer = Color(0xFF410002),
)

private val DarkColors = darkColorScheme(
    primary = Color(0xFFDCC6A0),
    onPrimary = Color(0xFF3C2F13),
    primaryContainer = Color(0xFF55452A),
    onPrimaryContainer = Color(0xFFF8E6C8),
    secondary = Color(0xFFCDC7B4),
    onSecondary = Color(0xFF343024),
    secondaryContainer = Color(0xFF4B4639),
    onSecondaryContainer = Color(0xFFEAE2D2),
    tertiary = Color(0xFFA5CEBC),
    onTertiary = Color(0xFF0C372A),
    tertiaryContainer = Color(0xFF274E40),
    onTertiaryContainer = Color(0xFFC1EAD7),
    surface = Color(0xFF151310),
    onSurface = Color(0xFFEAE1D9),
    surfaceVariant = Color(0xFF4A463A),
    onSurfaceVariant = Color(0xFFCCC6B8),
    surfaceContainerLowest = Color(0xFF0F0E0B),
    surfaceContainerLow = Color(0xFF1D1B17),
    surfaceContainer = Color(0xFF211F1B),
    surfaceContainerHigh = Color(0xFF2C2925),
    surfaceContainerHighest = Color(0xFF373430),
    outline = Color(0xFF958E80),
    outlineVariant = Color(0xFF4A463A),
    error = Color(0xFFFFB4AB),
    onError = Color(0xFF690005),
    errorContainer = Color(0xFF93000A),
    onErrorContainer = Color(0xFFFFDAD6),
)

private val MistShapes = Shapes(
    extraSmall = androidx.compose.foundation.shape.RoundedCornerShape(8.dp),
    small = androidx.compose.foundation.shape.RoundedCornerShape(12.dp),
    medium = androidx.compose.foundation.shape.RoundedCornerShape(16.dp),
    large = androidx.compose.foundation.shape.RoundedCornerShape(24.dp),
    extraLarge = androidx.compose.foundation.shape.RoundedCornerShape(32.dp),
)

private val MistTypography = Typography().let { base ->
    base.copy(
        headlineMedium = base.headlineMedium.copy(
            fontWeight = FontWeight.SemiBold,
            letterSpacing = (-0.4).sp,
        ),
        headlineSmall = base.headlineSmall.copy(
            fontWeight = FontWeight.SemiBold,
            letterSpacing = (-0.3).sp,
        ),
        titleLarge = base.titleLarge.copy(fontWeight = FontWeight.SemiBold),
        labelLarge = TextStyle(
            fontSize = 14.sp,
            lineHeight = 20.sp,
            fontWeight = FontWeight.Medium,
            letterSpacing = 0.1.sp,
        ),
    )
}

/** Application theme. Uses the Mist sand accent in light and dark. */
@Composable
internal fun MistTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    content: @Composable () -> Unit,
) {
    MaterialTheme(
        colorScheme = if (darkTheme) DarkColors else LightColors,
        typography = MistTypography,
        shapes = MistShapes,
        content = content,
    )
}
