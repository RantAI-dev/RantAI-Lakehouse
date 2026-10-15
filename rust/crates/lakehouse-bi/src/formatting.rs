//! The closed lists behind the deployment's formatting settings (`BI-17`
//! part A, decision D5): the number locales, the fonts the console ships, and
//! the one hex-colour check every colour field uses.
//!
//! Why here and not in the API: the settings route, the dashboard theme
//! (`Board.font`) and the chart style validation all need the same lists, and
//! a list kept in three places drifts. Migration `0072` repeats the lists in
//! its `CHECK`s as the backstop for shape; the settings route test saves every
//! value of these lists through the table, so a drift fails there.

/// The number locales a deployment may choose (feature decision 2).
pub const NUMBER_LOCALES: [&str; 2] = ["en-US", "id-ID"];

/// The fonts the console ships (feature decision 12); nothing is loaded from
/// the internet at run time. `system` is the operating system's own font.
pub const SHIPPED_FONTS: [&str; 5] = [
    "inter",
    "ibm-plex-sans",
    "source-serif",
    "jetbrains-mono",
    "system",
];

/// At least and at most how many colours a company palette holds.
pub const COMPANY_PALETTE_LEN: std::ops::RangeInclusive<usize> = 2..=12;

/// Whether `s` is `#rrggbb` (six hex digits, either case).
#[must_use]
pub fn is_hex_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s.bytes().skip(1).all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hex_colour_is_a_hash_and_six_hex_digits() {
        assert!(is_hex_color("#1a2B3c"));
        for bad in ["", "#fff", "1a2b3c", "#1a2b3g", "#1a2b3c4", "red"] {
            assert!(!is_hex_color(bad), "{bad}");
        }
    }

    #[test]
    fn the_lists_have_no_duplicates() {
        for list in [&NUMBER_LOCALES[..], &SHIPPED_FONTS[..]] {
            let mut sorted = list.to_vec();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(sorted.len(), list.len());
        }
    }
}
