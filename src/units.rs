//! Display units. Patterns are always stored in millimetres; this converts
//! for display and parses typed lengths.

use serde::{Deserialize, Serialize};

pub const MM_PER_INCH: f64 = 25.4;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Units {
    #[default]
    Millimeters,
    Inches,
}

impl Units {
    pub fn name(self) -> &'static str {
        match self {
            Units::Millimeters => "Millimetres",
            Units::Inches => "Inches",
        }
    }

    pub fn toggled(self) -> Self {
        match self {
            Units::Millimeters => Units::Inches,
            Units::Inches => Units::Millimeters,
        }
    }

    pub fn display_value(self, mm: f64) -> f64 {
        match self {
            Units::Millimeters => mm,
            Units::Inches => mm / MM_PER_INCH,
        }
    }

    pub fn to_mm(self, v: f64) -> f64 {
        match self {
            Units::Millimeters => v,
            Units::Inches => v * MM_PER_INCH,
        }
    }

    /// Suffix for values in this unit, including the leading space.
    pub fn suffix(self) -> &'static str {
        match self {
            Units::Millimeters => " mm",
            Units::Inches => " in",
        }
    }

    /// Decimals shown for lengths (0.1 mm, 0.01 in ≈ 0.25 mm).
    pub fn decimals(self) -> usize {
        match self {
            Units::Millimeters => 1,
            Units::Inches => 2,
        }
    }

    /// Drag speed for length fields, in display units per pixel.
    pub fn drag_speed(self) -> f64 {
        match self {
            Units::Millimeters => 0.5,
            Units::Inches => 0.02,
        }
    }

    /// Number formatted to this unit's precision, without suffix.
    pub fn number(self, mm: f64) -> String {
        format!("{:.*}", self.decimals(), self.display_value(mm))
    }

    pub fn format_area(self, mm2: f64) -> String {
        match self {
            Units::Millimeters => format!("{:.1} cm²", mm2 / 100.0),
            Units::Inches => format!("{:.1} in²", mm2 / (MM_PER_INCH * MM_PER_INCH)),
        }
    }

    /// Default snap grid step for this unit, in mm (10 mm / ⅛ in).
    pub fn default_snap_step(self) -> f64 {
        match self {
            Units::Millimeters => 10.0,
            Units::Inches => MM_PER_INCH / 8.0,
        }
    }

    /// Arrow-key nudge steps `(normal, fine)` in mm when not snapping to the grid.
    pub fn nudge_steps(self) -> (f64, f64) {
        match self {
            Units::Millimeters => (10.0, 1.0),
            Units::Inches => (MM_PER_INCH / 4.0, MM_PER_INCH / 16.0),
        }
    }

    /// Grid spacing `(minor, major)` in mm for `zoom` px/mm: the smallest
    /// "nice" step at least `min_px` wide, and a coarser step for emphasis.
    pub fn grid_steps(self, zoom: f64, min_px: f64) -> (f64, f64) {
        match self {
            Units::Millimeters => {
                let mut step = 1.0;
                while step * zoom < min_px {
                    step *= 10.0;
                }
                (step, step * 10.0)
            }
            Units::Inches => {
                const MINOR: [f64; 10] = [0.0625, 0.125, 0.25, 0.5, 1.0, 2.0, 6.0, 12.0, 36.0, 120.0];
                const MAJOR: [f64; 4] = [1.0, 12.0, 36.0, 360.0];
                let minor = MINOR
                    .into_iter()
                    .find(|s| s * MM_PER_INCH * zoom >= min_px)
                    .unwrap_or(360.0);
                let major = MAJOR.into_iter().find(|&m| m > minor).unwrap_or(minor * 10.0);
                (minor * MM_PER_INCH, major * MM_PER_INCH)
            }
        }
    }

    /// Parse a typed length in this unit to mm. Accepts decimals (`12.5`),
    /// fractions (`3/8`) and mixed numbers (`3 1/2`, `3-1/2`).
    pub fn parse(self, s: &str) -> Option<f64> {
        let s = s.trim();
        let value = if let Some((whole, frac)) = s.split_once([' ', '-']) {
            let whole: f64 = whole.trim().parse().ok()?;
            whole + parse_fraction(frac.trim())?
        } else if s.contains('/') {
            parse_fraction(s)?
        } else {
            s.parse().ok()?
        };
        value.is_finite().then(|| self.to_mm(value))
    }
}

/// How lengths in inches are written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum InchFormat {
    Eighths,
    /// Tape-measure precision, the default.
    #[default]
    Sixteenths,
    ThirtySeconds,
    Decimal,
}

impl InchFormat {
    pub const ALL: [InchFormat; 4] =
        [InchFormat::Eighths, InchFormat::Sixteenths, InchFormat::ThirtySeconds, InchFormat::Decimal];

    pub fn name(self) -> &'static str {
        match self {
            InchFormat::Eighths => "Fractions to 1/8",
            InchFormat::Sixteenths => "Fractions to 1/16",
            InchFormat::ThirtySeconds => "Fractions to 1/32",
            InchFormat::Decimal => "Decimal",
        }
    }

    fn denominator(self) -> Option<u64> {
        match self {
            InchFormat::Eighths => Some(8),
            InchFormat::Sixteenths => Some(16),
            InchFormat::ThirtySeconds => Some(32),
            InchFormat::Decimal => None,
        }
    }
}

/// Display units together with how inches are written: formats lengths for
/// people to read. (Agents get plain decimals in the display unit instead.)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lengths {
    pub units: Units,
    pub inches: InchFormat,
}

impl Lengths {
    /// Fraction denominator, when lengths are written as fractions.
    fn denominator(self) -> Option<u64> {
        match self.units {
            Units::Inches => self.inches.denominator(),
            Units::Millimeters => None,
        }
    }

    /// Number without suffix: `"123.4"`, `"23 5/8"` or `"23.62"`.
    pub fn number(self, mm: f64) -> String {
        match self.denominator() {
            Some(den) => fraction(self.units.display_value(mm), den),
            None => self.units.number(mm),
        }
    }

    /// `"123.4 mm"`, `"23 5/8 in"` or `"23.62 in"`.
    pub fn format(self, mm: f64) -> String {
        format!("{}{}", self.number(mm), self.units.suffix())
    }

    /// Same number as [`Self::number`], from a value already in display units.
    pub fn number_display(self, v: f64) -> String {
        self.number(self.units.to_mm(v))
    }

    pub fn suffix(self) -> &'static str {
        self.units.suffix()
    }

    pub fn parse(self, s: &str) -> Option<f64> {
        self.units.parse(s)
    }
}

/// `v` rounded to the nearest `1/den` and written as a mixed number in
/// lowest terms: `"23 5/8"`, `"-1/16"`, `"4"`.
fn fraction(v: f64, den: u64) -> String {
    let n = (v.abs() * den as f64).round() as u64;
    let (whole, mut num, mut d) = (n / den, n % den, den);
    while num != 0 && num % 2 == 0 && d % 2 == 0 {
        num /= 2;
        d /= 2;
    }
    let sign = if v < 0.0 && n != 0 { "-" } else { "" };
    match (whole, num) {
        (w, 0) => format!("{sign}{w}"),
        (0, _) => format!("{sign}{num}/{d}"),
        (w, _) => format!("{sign}{w} {num}/{d}"),
    }
}

fn parse_fraction(s: &str) -> Option<f64> {
    let (num, den) = s.split_once('/')?;
    let (num, den): (f64, f64) = (num.trim().parse().ok()?, den.trim().parse().ok()?);
    (den != 0.0).then(|| num / den)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversion_and_format() {
        assert_eq!(Units::Inches.number(25.4), "1.00");
        assert_eq!(Units::Millimeters.number(12.34), "12.3");
        assert_eq!(Units::Inches.format_area(25.4 * 25.4 * 3.0), "3.0 in²");
        assert_eq!(Units::Millimeters.format_area(250.0), "2.5 cm²");
    }

    #[test]
    fn parse_lengths() {
        let mm = Units::Millimeters;
        let inch = Units::Inches;
        assert_eq!(mm.parse("12.5"), Some(12.5));
        assert_eq!(inch.parse("2"), Some(50.8));
        assert_eq!(inch.parse("3/8"), Some(0.375 * 25.4));
        assert_eq!(inch.parse("3 1/2"), Some(3.5 * 25.4));
        assert_eq!(inch.parse("3-1/2"), Some(3.5 * 25.4));
        assert_eq!(inch.parse("1/0"), None);
        assert_eq!(inch.parse("abc"), None);
        assert_eq!(inch.parse(""), None);
    }

    #[test]
    fn inch_fractions() {
        let l = Lengths { units: Units::Inches, inches: InchFormat::Sixteenths };
        assert_eq!(l.format(23.625 * 25.4), "23 5/8 in");
        assert_eq!(l.number(0.0625 * 25.4), "1/16");
        assert_eq!(l.number(-0.0625 * 25.4), "-1/16");
        assert_eq!(l.number(4.0 * 25.4), "4");
        assert_eq!(l.number(0.01 * 25.4), "0"); // rounds to zero, no "-0"
        assert_eq!(l.number(-0.01 * 25.4), "0");
        assert_eq!(l.number(2.97 * 25.4), "3"); // carries into the whole number
        let eighths = Lengths { inches: InchFormat::Eighths, ..l };
        assert_eq!(eighths.number(0.3 * 25.4), "1/4");
        let decimal = Lengths { inches: InchFormat::Decimal, ..l };
        assert_eq!(decimal.format(23.62 * 25.4), "23.62 in");
        // Millimetres ignore the inch format; fractions parse back.
        let mm = Lengths { units: Units::Millimeters, ..l };
        assert_eq!(mm.format(12.34), "12.3 mm");
        assert_eq!(l.parse(&l.number(23.625 * 25.4)), Some(23.625 * 25.4));
    }

    #[test]
    fn grid_steps_are_nice() {
        // 1 px/mm: 10 mm minor, 100 mm major; inches: ½ in minor (12.7 px), 1 in major.
        assert_eq!(Units::Millimeters.grid_steps(1.0, 8.0), (10.0, 100.0));
        assert_eq!(Units::Inches.grid_steps(1.0, 8.0), (12.7, 25.4));
        // Zoomed far out the inch grid steps up to feet and yards.
        let (minor, major) = Units::Inches.grid_steps(0.05, 8.0);
        assert!((minor / 25.4 - 12.0).abs() < 1e-9 && (major / 25.4 - 36.0).abs() < 1e-9);
    }
}
