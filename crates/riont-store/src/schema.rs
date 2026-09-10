//! WPILib `structSchema` property parser.
//!
//! Struct-typed NT topics announce their layout in the `structSchema`
//! property, e.g.:
//!
//! ```text
//! Pose2d{Translation2d{x:double, y:double}, Rotation2d{radians:double}}
//! ```
//!
//! [`parse_struct_schema`] flattens that grammar into the ordered leaf
//! `(name, type)` pairs the inspector shows for undecoded struct topics.
//! Nesting is matched brace for brace and arbitrary whitespace between
//! tokens is accepted. Malformed input yields an EMPTY list (never a
//! panic) — callers degrade to showing the raw schema string.

/// One flattened leaf of a struct schema: field name + primitive type
/// (e.g. `radians`, `double`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaLeaf {
    pub name: String,
    pub ty: String,
}

/// Recursion guard for deeply nested (or adversarial) schema strings.
const MAX_DEPTH: usize = 16;

/// Parse a `structSchema` property string into ordered leaf fields.
/// Returns an empty vector when the schema is missing, empty, or
/// malformed — there is no partial credit: a schema we cannot fully
/// parse is no schema at all.
pub fn parse_struct_schema(schema: &str) -> Vec<SchemaLeaf> {
    let mut p = Parser {
        b: schema.as_bytes(),
        i: 0,
    };
    p.skip_ws();
    // Outer wrapper type name (`Pose2d` in the example above): not a leaf.
    if p.take_token().is_empty() {
        return Vec::new();
    }
    p.skip_ws();
    if p.peek() != Some(b'{') {
        return Vec::new();
    }
    p.i += 1;
    let mut out = Vec::new();
    if !p.fields(&mut out, 1) {
        return Vec::new();
    }
    p.skip_ws();
    // Trailing junk after the closing brace: malformed.
    if p.i != p.b.len() {
        return Vec::new();
    }
    out
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.i += 1;
        }
    }

    /// A token: any run of characters that is not a delimiter
    /// (`,` `{` `}` `:`) or whitespace. Used for both field names and
    /// primitive type names.
    fn take_token(&mut self) -> String {
        let start = self.i;
        while let Some(c) = self.peek() {
            if matches!(c, b',' | b'{' | b'}' | b':' | b' ' | b'\t' | b'\r' | b'\n') {
                break;
            }
            self.i += 1;
        }
        String::from_utf8_lossy(&self.b[start..self.i]).into_owned()
    }

    /// Parse fields until the matching `}` (consumed). `depth` bounds
    /// recursion; a false return means malformed.
    fn fields(&mut self, out: &mut Vec<SchemaLeaf>, depth: usize) -> bool {
        if depth > MAX_DEPTH {
            return false;
        }
        loop {
            self.skip_ws();
            let name = self.take_token();
            if name.is_empty() {
                return false;
            }
            self.skip_ws();
            match self.peek() {
                // Nested struct field: recurse into its fields.
                Some(b'{') => {
                    self.i += 1;
                    if !self.fields(out, depth + 1) {
                        return false;
                    }
                }
                // Leaf field: `name:type`. The type may itself name a
                // nested struct (`angle:Rotation2d{radians:double}`) —
                // then recurse instead of stopping at the token; the
                // innermost leaf name is what gets flattened out.
                Some(b':') => {
                    self.i += 1;
                    self.skip_ws();
                    let ty = self.take_token();
                    if ty.is_empty() {
                        return false;
                    }
                    self.skip_ws();
                    if self.peek() == Some(b'{') {
                        self.i += 1;
                        if !self.fields(out, depth + 1) {
                            return false;
                        }
                    } else {
                        out.push(SchemaLeaf { name, ty });
                    }
                }
                _ => return false,
            }
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    return true;
                }
                _ => return false,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaves(v: &[(&str, &str)]) -> Vec<SchemaLeaf> {
        v.iter()
            .map(|(n, t)| SchemaLeaf {
                name: n.to_string(),
                ty: t.to_string(),
            })
            .collect()
    }

    #[test]
    fn flat_schema_parses() {
        let out = parse_struct_schema("Point{x:double, y:double}");
        assert_eq!(out, leaves(&[("x", "double"), ("y", "double")]));
    }

    #[test]
    fn nested_schema_flattens_in_order() {
        let out = parse_struct_schema(
            "Pose2d{Translation2d{x:double, y:double}, Rotation2d{radians:double}}",
        );
        assert_eq!(
            out,
            leaves(&[("x", "double"), ("y", "double"), ("radians", "double")])
        );
    }

    #[test]
    fn nested_then_leaf_mixes() {
        // Real WPILib SwerveModuleState shape: struct field then leaf.
        let out = parse_struct_schema(
            "SwerveModuleState{angle:Rotation2d{radians:double}, \
             speedMetersPerSecond:double}",
        );
        assert_eq!(
            out,
            leaves(&[("radians", "double"), ("speedMetersPerSecond", "double")])
        );
    }

    #[test]
    fn whitespace_variants_parse() {
        let out = parse_struct_schema(
            "Pose2d { Translation2d { x:double ,  y:double } ,\n\t \
             Rotation2d{ radians : double } }",
        );
        assert_eq!(
            out,
            leaves(&[("x", "double"), ("y", "double"), ("radians", "double")])
        );
    }

    #[test]
    fn non_double_types_parse() {
        let out = parse_struct_schema("S{ok:boolean, id:int64, note:char, temp:float, raw:uint8}");
        assert_eq!(
            out,
            leaves(&[
                ("ok", "boolean"),
                ("id", "int64"),
                ("note", "char"),
                ("temp", "float"),
                ("raw", "uint8"),
            ])
        );
    }

    #[test]
    fn single_leaf_schema_parses() {
        assert_eq!(
            parse_struct_schema("Angle{radians:double}"),
            leaves(&[("radians", "double")])
        );
    }

    #[test]
    fn malformed_missing_brace_is_empty() {
        assert!(parse_struct_schema("x:double, y:double").is_empty());
    }

    #[test]
    fn malformed_missing_colon_is_empty() {
        assert!(parse_struct_schema("S{x double, y:double}").is_empty());
    }

    #[test]
    fn malformed_unbalanced_braces_are_empty() {
        assert!(parse_struct_schema("S{x:double, y:double").is_empty());
        assert!(parse_struct_schema("S{x:double}}").is_empty());
    }

    #[test]
    fn malformed_trailing_junk_is_empty() {
        assert!(parse_struct_schema("S{x:double} garbage").is_empty());
    }

    #[test]
    fn malformed_empty_and_garbage_are_empty() {
        assert!(parse_struct_schema("").is_empty());
        assert!(parse_struct_schema("   ").is_empty());
        assert!(parse_struct_schema("{}").is_empty());
        assert!(parse_struct_schema("S{}").is_empty());
        assert!(parse_struct_schema("S{x:}").is_empty());
        assert!(parse_struct_schema("S{,}").is_empty());
        assert!(parse_struct_schema("S{x:double,,y:double}").is_empty());
    }

    #[test]
    fn deep_nesting_bounded_not_panicking() {
        let mut s = String::new();
        for _ in 0..64 {
            s.push_str("N{a:");
        }
        for _ in 0..64 {
            s.push_str("double}");
        }
        // Past MAX_DEPTH the schema is refused, never a stack overflow.
        assert!(parse_struct_schema(&s).is_empty());
    }
}
