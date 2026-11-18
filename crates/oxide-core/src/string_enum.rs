//! Internal helpers that keep one declaration per fact.

/// Generate the string view of a state enum from one table.
///
/// Every state enum in the domain is stored, logged and plotted by its
/// `snake_case` name: `serde` writes it into the event log, `parse` reads it
/// back out of PostgreSQL, and `as_str` names the column value. Written by
/// hand, those are three tables that drift the moment a variant is added —
/// and a missing `parse` arm is silent, because the row mapper falls back to a
/// default state. The macro makes the variant list appear twice at most (the
/// `serde` attribute is still needed to derive `Serialize`/`Deserialize`), in
/// the same screen, so drift is visible on review.
///
/// The caller keeps the `enum` declaration (and its derives); the macro adds
/// only the conversions.
macro_rules! string_enum {
    ($name:ident {
        $($variant:ident => $text:literal),+ $(,)?
    }) => {
        impl $name {
            /// The `snake_case` name used in the event log, the database and
            /// the API.
            #[must_use]
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text),+
                }
            }

            /// The variant whose `as_str` is `value`.
            #[must_use]
            pub fn parse(value: &str) -> Option<Self> {
                match value {
                    $($text => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }
    };
}

pub(crate) use string_enum;
