//! The function catalog: the single owner of which functions exist, their
//! arity, signature, category and one line of help. The compiler checks
//! arity against it, `GET /api/dashboard/calc-fields/functions` serves it to
//! the console (suggestions, the help line) and the assistant reads it as
//! its list of what a formula may call. A function that is not here does not
//! compile, and a function here that [`super::compile`] does not know fails
//! the test that compiles every entry's example.
//!
//! Names are matched without regard to case.

use serde::Serialize;

/// One function.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FnInfo {
    /// The name as it is written in the catalog.
    pub name: &'static str,
    /// `maths`, `text`, `dates`, `conditions`, `conversion` or `aggregations`.
    pub category: &'static str,
    /// The call with its arguments, for the help line.
    pub signature: &'static str,
    /// One sentence.
    pub help: &'static str,
    /// A formula that uses it, over the sample columns of the tests
    /// (`amount`, `qty`, `label`, `day`, `ts`).
    pub example: &'static str,
    /// Fewest arguments.
    pub min_args: usize,
    /// Most arguments; `None` for no limit.
    pub max_args: Option<usize>,
    /// Whether the function aggregates rows.
    pub aggregate: bool,
}

const fn f(
    name: &'static str,
    category: &'static str,
    signature: &'static str,
    help: &'static str,
    example: &'static str,
    args: (usize, Option<usize>),
) -> FnInfo {
    FnInfo {
        name,
        category,
        signature,
        help,
        example,
        min_args: args.0,
        max_args: args.1,
        aggregate: false,
    }
}

const fn agg(
    name: &'static str,
    signature: &'static str,
    help: &'static str,
    example: &'static str,
    args: (usize, Option<usize>),
) -> FnInfo {
    FnInfo {
        name,
        category: "aggregations",
        signature,
        help,
        example,
        min_args: args.0,
        max_args: args.1,
        aggregate: true,
    }
}

/// Every function, grouped by category in the order the console lists them.
// One row per function; rustfmt would spread each over eight lines.
#[rustfmt::skip]
pub static CATALOG: &[FnInfo] = &[
    f("Abs", "maths", "Abs(x)", "The absolute value of a number.", "Abs([amount] - 100)", (1, Some(1))),
    f("Round", "maths", "Round(x, digits)", "Round to a whole number, or to 'digits' decimals (a number from -10 to 10 written out); a half goes to the even neighbour, so 2.5 is 2.", "Round([amount] / [qty], 2)", (1, Some(2))),
    f("Floor", "maths", "Floor(x)", "The largest whole number not above x.", "Floor([amount])", (1, Some(1))),
    f("Ceil", "maths", "Ceil(x)", "The smallest whole number not below x.", "Ceil([amount])", (1, Some(1))),
    f("Power", "maths", "Power(x, y)", "x to the power y.", "Power([qty], 2)", (2, Some(2))),
    f("Sqrt", "maths", "Sqrt(x)", "The square root; empty for a negative number.", "Sqrt([amount])", (1, Some(1))),
    f("Exp", "maths", "Exp(x)", "e to the power x.", "Exp([qty] / 100)", (1, Some(1))),
    f("Log", "maths", "Log(x)", "The natural logarithm; empty for zero or a negative number.", "Log([amount])", (1, Some(1))),
    f("Mod", "maths", "Mod(x, y)", "The remainder of x divided by y; empty when y is zero.", "Mod([qty], 7)", (2, Some(2))),
    f("Greatest", "maths", "Greatest(x, y, ...)", "The largest of two or more numbers.", "Greatest([amount], [qty], 0)", (2, None)),
    f("Least", "maths", "Least(x, y, ...)", "The smallest of two or more numbers.", "Least([amount], [qty], 100)", (2, None)),
    f("Concat", "text", "Concat(a, b, ...)", "Join values into one text; an empty value counts as nothing.", "Concat([label], '-', [qty])", (2, None)),
    f("Upper", "text", "Upper(text)", "Capital letters.", "Upper([label])", (1, Some(1))),
    f("Lower", "text", "Lower(text)", "Small letters.", "Lower([label])", (1, Some(1))),
    f("Trim", "text", "Trim(text)", "Remove spaces at both ends.", "Trim([label])", (1, Some(1))),
    f("Length", "text", "Length(text)", "The number of characters.", "Length([label])", (1, Some(1))),
    f("Substring", "text", "Substring(text, start, length)", "Part of a text; 'start' counts from 1, 'length' is optional.", "Substring([label], 2, 3)", (2, Some(3))),
    f("Replace", "text", "Replace(text, find, with)", "Replace every occurrence of 'find'.", "Replace([label], 'a', 'b')", (3, Some(3))),
    f("Contains", "text", "Contains(text, part)", "True when the text has 'part' in it (capitals matter).", "Contains([label], 'ab')", (2, Some(2))),
    f("StartsWith", "text", "StartsWith(text, part)", "True when the text begins with 'part'.", "StartsWith([label], 'ab')", (2, Some(2))),
    f("EndsWith", "text", "EndsWith(text, part)", "True when the text ends with 'part'.", "EndsWith([label], 'ab')", (2, Some(2))),
    f("Left", "text", "Left(text, n)", "The first n characters.", "Left([label], 3)", (2, Some(2))),
    f("Right", "text", "Right(text, n)", "The last n characters.", "Right([label], 3)", (2, Some(2))),
    f("Year", "dates", "Year(date)", "The year of a date or timestamp (in the report time zone).", "Year([day])", (1, Some(1))),
    f("Month", "dates", "Month(date)", "The month, 1 to 12.", "Month([day])", (1, Some(1))),
    f("Day", "dates", "Day(date)", "The day of the month, 1 to 31.", "Day([day])", (1, Some(1))),
    f("Hour", "dates", "Hour(timestamp)", "The hour, 0 to 23, of a timestamp.", "Hour([ts])", (1, Some(1))),
    f("Weekday", "dates", "Weekday(date)", "The day of the week, 1 (Monday) to 7 (Sunday).", "Weekday([day])", (1, Some(1))),
    f("DateTrunc", "dates", "DateTrunc(unit, date)", "The start of the minute, hour, day, week, month, quarter or year.", "DateTrunc('month', [day])", (2, Some(2))),
    f("DateAdd", "dates", "DateAdd(unit, n, date)", "Add n minutes, hours, days, weeks, months, quarters or years.", "DateAdd('day', 7, [day])", (3, Some(3))),
    f("DateDiff", "dates", "DateDiff(unit, from, to)", "How many unit boundaries lie between the two (months from 31 Jan to 1 Feb is 1); negative when 'to' is before 'from'.", "DateDiff('day', [day], Today())", (3, Some(3))),
    f("Today", "dates", "Today()", "Today's date in the report time zone.", "DateDiff('day', [day], Today())", (0, Some(0))),
    f("Now", "dates", "Now()", "The current time in the report time zone.", "DateDiff('hour', [ts], Now())", (0, Some(0))),
    f("If", "conditions", "If(test, then, else)", "'then' when the test is true, otherwise 'else'.", "If([amount] > 100, 'big', 'small')", (3, Some(3))),
    f("Case", "conditions", "Case(test, value, ..., else)", "The value of the first test that is true, or the last argument.", "Case([amount] > 100, 'big', [amount] > 10, 'mid', 'small')", (3, None)),
    f("Coalesce", "conditions", "Coalesce(a, b, ...)", "The first value that is not empty.", "Coalesce([label], 'none')", (2, None)),
    f("IsNull", "conditions", "IsNull(x)", "True when the value is empty.", "IsNull([label])", (1, Some(1))),
    f("Between", "conditions", "Between(x, low, high)", "True when x is from low to high, both included.", "Between([amount], 10, 100)", (3, Some(3))),
    f("ToNumber", "conversion", "ToNumber(text)", "Read a text as a number; empty when it is not one.", "ToNumber([label])", (1, Some(1))),
    f("ToText", "conversion", "ToText(x)", "A number, date or true/false as text.", "ToText([amount])", (1, Some(1))),
    f("ToDate", "conversion", "ToDate(x)", "Read a text such as 2026-03-01 as a date, or take the date of a timestamp.", "ToDate('2026-03-01')", (1, Some(1))),
    agg("Sum", "Sum(x)", "The total of a number over the rows of each group.", "Sum([amount])", (1, Some(1))),
    agg("Count", "Count(x)", "The number of rows (or, with x, of rows where x is not empty).", "Count([label])", (0, Some(1))),
    agg("CountDistinct", "CountDistinct(x)", "The number of different values.", "CountDistinct([label])", (1, Some(1))),
    agg("Avg", "Avg(x)", "The average of a number.", "Avg([amount])", (1, Some(1))),
    agg("Median", "Median(x)", "The middle value of a number.", "Median([amount])", (1, Some(1))),
    agg("Percentile", "Percentile(x, p)", "The value below which a share p (0 to 1, written out) of the rows fall.", "Percentile([amount], 0.9)", (2, Some(2))),
    agg("Min", "Min(x)", "The smallest value of a number, text or date.", "Min([day])", (1, Some(1))),
    agg("Max", "Max(x)", "The largest value of a number, text or date.", "Max([amount])", (1, Some(1))),
    agg("StdDev", "StdDev(x)", "The sample standard deviation of a number.", "StdDev([amount])", (1, Some(1))),
    agg("SumIf", "SumIf(x, test)", "The total of x over the rows where the test is true.", "SumIf([amount], [qty] > 1)", (2, Some(2))),
    agg("CountIf", "CountIf(test)", "The number of rows where the test is true.", "CountIf([amount] > 100)", (1, Some(1))),
    agg("AvgIf", "AvgIf(x, test)", "The average of x over the rows where the test is true.", "AvgIf([amount], [qty] > 1)", (2, Some(2))),
];

/// The function called `name`, ignoring case.
#[must_use]
pub fn lookup(name: &str) -> Option<&'static FnInfo> {
    CATALOG.iter().find(|f| f.name.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_lookup_ignores_case() {
        let mut names: Vec<_> = CATALOG
            .iter()
            .map(|f| f.name.to_ascii_lowercase())
            .collect();
        names.sort();
        let n = names.len();
        names.dedup();
        assert_eq!(names.len(), n, "a function is listed twice");
        assert_eq!(lookup("sUm").map(|f| f.name), Some("Sum"));
        assert!(lookup("Sleep").is_none());
        assert_eq!(n, 53);
    }

    #[test]
    fn every_entry_has_a_signature_that_starts_with_its_name_and_some_help() {
        for f in CATALOG {
            assert!(f.signature.starts_with(f.name), "{}", f.name);
            assert!(f.help.ends_with('.'), "{}", f.name);
            assert!(f.max_args.is_none_or(|m| m >= f.min_args), "{}", f.name);
        }
    }
}
