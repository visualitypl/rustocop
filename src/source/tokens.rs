//! RuboCop's `processed_source.tokens`: Prism 1.9's own token stream
//! (`parser::tokens`) projected like `Prism::Translation::Parser::Lexer#to_a`
//! (prism 1.9.0, MIT): the same skipped tokens, merges, splits, out-of-order
//! newline tokens and parser-gem type names. Only positions and types are
//! kept; token values are never unescaped or evaluated.
use ruby_prism_sys as sys;

use crate::parser::tokens::RawToken;

use super::SourceFile;

/// A parser-gem token: its type name (`tIDENTIFIER`, `tEQL`, …), byte range
/// and the 1-based line plus 0-based character column of its start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Token {
    pub kind: &'static str,
    pub start: usize,
    pub end: usize,
    pub line: u32,
    pub column: u32,
    /// The 1-based line and 0-based character column of the end.
    pub end_line: u32,
    pub end_column: u32,
}

impl Token {
    pub fn is_comment(&self) -> bool {
        self.kind == "tCOMMENT"
    }

    pub fn is_newline(&self) -> bool {
        self.kind == "tNL"
    }

    /// rubocop-ast `Token#equal_sign?`.
    pub fn is_equal_sign(&self) -> bool {
        matches!(self.kind, "tEQL" | "tOP_ASGN")
    }
}

/// `PrecedingFollowingAlignment::ASSIGNMENT_OR_COMPARISON_TOKENS`.
pub(crate) fn assignment_or_comparison(kind: &str) -> bool {
    matches!(
        kind,
        "tEQL" | "tEQ" | "tEQQ" | "tNEQ" | "tLEQ" | "tGEQ" | "tOP_ASGN" | "tLSHFT"
    )
}

pub(super) fn collect(file: &SourceFile) -> Vec<Token> {
    let raw = crate::parser::tokens::lex(&file.bytes);
    let spans = translate(&file.bytes, &raw, |offset| file.line_of(offset));
    let positions = file.line_cols(spans.iter().flat_map(|s| [s.1, s.2]));
    spans
        .into_iter()
        .zip(positions.as_chunks::<2>().0)
        .map(|((kind, start, end), position)| Token {
            kind,
            start,
            end,
            line: position[0].0,
            column: position[0].1 - 1,
            end_line: position[1].0,
            end_column: position[1].1 - 1,
        })
        .collect()
}

const EXPR_BEG: u32 = sys::PM_LEX_STATE_BEG;
const EXPR_LABEL: u32 = sys::PM_LEX_STATE_LABEL;

const LPAREN_CONVERSION: &[&str] = &[
    "kBREAK",
    "tCARET",
    "kCASE",
    "tDIVIDE",
    "kFOR",
    "kIF",
    "kNEXT",
    "kRETURN",
    "kUNTIL",
    "kWHILE",
    "tAMPER",
    "tANDOP",
    "tBANG",
    "tCOMMA",
    "tDOT2",
    "tDOT3",
    "tEQL",
    "tLPAREN",
    "tLPAREN2",
    "tLPAREN_ARG",
    "tLSHFT",
    "tNL",
    "tOP_ASGN",
    "tOROP",
    "tPIPE",
    "tSEMI",
    "tSTRING_DBEG",
    "tUMINUS",
    "tUPLUS",
    "tLCURLY",
];

fn continuation(kind: Option<sys::pm_token_type>) -> bool {
    matches!(
        kind,
        Some(sys::PM_TOKEN_COMMENT | sys::PM_TOKEN_AMPERSAND_DOT | sys::PM_TOKEN_DOT)
    )
}

fn percent_array(quote: Option<&[u8]>) -> bool {
    quote.is_some_and(|q| {
        [b"%w", b"%W", b"%i", b"%I"]
            .iter()
            .any(|prefix| q.starts_with(*prefix))
    })
}

fn interpolation(quote: Option<&[u8]>) -> bool {
    quote.is_some_and(|q| {
        !q.ends_with(b"'")
            && ![b"%q", b"%w", b"%i", b"%s"]
                .iter()
                .any(|p| q.starts_with(*p))
    })
}

/// `simplify_string?`: a `'…'` without newlines, or a `"…"` whose every
/// newline is escaped.
fn simplify(content: &[u8], quote: &[u8]) -> bool {
    match quote {
        b"'" => !content.contains(&b'\n'),
        b"\"" => content.split_inclusive(|&b| b == b'\n').all(|line| {
            line.strip_suffix(b"\n")
                .is_none_or(|body| body.iter().rev().take_while(|&&b| b == b'\\').count() % 2 == 1)
        }),
        _ => false,
    }
}

/// The translator's `to_a`, over byte offsets: `(type, start, end)`.
fn translate(
    bytes: &[u8],
    lexed: &[RawToken],
    line_of: impl Fn(usize) -> u32,
) -> Vec<(&'static str, usize, usize)> {
    let mut tokens: Vec<(&'static str, usize, usize)> = Vec::new();
    let mut quotes: Vec<&[u8]> = Vec::new();
    let mut comment_newline: Option<(usize, usize)> = None;
    let kind_at = |index: usize| lexed.get(index).map(|t| t.kind);
    let mut index = 0;
    while index < lexed.len() {
        let token = lexed[index];
        index += 1;
        if matches!(
            token.kind,
            sys::PM_TOKEN_IGNORED_NEWLINE | sys::PM_TOKEN___END__ | sys::PM_TOKEN_EOF
        ) {
            continue;
        }
        let Some(mut kind) = parser_type(token.kind) else {
            continue;
        };
        let value = &bytes[token.start..token.end];
        let (mut start, mut end) = (token.start, token.end);
        match kind {
            "kDO" => {
                if tokens
                    .iter()
                    .rev()
                    .find(|t| matches!(t.0, "kDO_LAMBDA" | "tLAMBDA" | "tLAMBEG"))
                    .is_some_and(|t| t.0 == "tLAMBDA")
                {
                    kind = "kDO_LAMBDA";
                }
            }
            "tCOMMENT" if token.kind == sys::PM_TOKEN_EMBDOC_BEGIN => {
                while index < lexed.len() - 1 && kind_at(index) != Some(sys::PM_TOKEN_EMBDOC_END) {
                    index += 1;
                }
                if let Some(last) = lexed.get(index) {
                    end = last.end;
                }
                index += 1;
            }
            "tCOMMENT" => {
                let chomped = value.ends_with(b"\n") || value.ends_with(b"\r");
                if chomped {
                    end -= 1;
                }
                let previous = index.checked_sub(2).and_then(|i| lexed.get(i));
                let next = kind_at(index);
                let inline = previous.is_some_and(|p| line_of(p.start) == line_of(token.start));
                let newline = (token.end.saturating_sub(1), token.end);
                if inline && chomped && !continuation(next) {
                    tokens.push((kind, start, end));
                    tokens.push(("tNL", newline.0, newline.1));
                    continue;
                } else if inline && next == Some(sys::PM_TOKEN_COMMENT) {
                    comment_newline = Some(newline);
                } else if let Some(saved) = comment_newline.filter(|_| !continuation(next)) {
                    tokens.push((kind, start, end));
                    tokens.push(("tNL", saved.0, saved.1));
                    comment_newline = None;
                    continue;
                }
            }
            "tNL" => {
                if kind_at(index) == Some(sys::PM_TOKEN_COMMENT) {
                    comment_newline = Some((start, end));
                    continue;
                }
            }
            "tINTEGER" if value.starts_with(b"+") => {
                tokens.push(("tUNARY_NUM", start, start + 1));
                start += 1;
            }
            "tLCURLY" if token.state == EXPR_BEG | EXPR_LABEL => kind = "tLBRACE",
            "tLPAREN2"
                if tokens
                    .last()
                    .is_none_or(|t| LPAREN_CONVERSION.contains(&t.0)) =>
            {
                kind = "tLPAREN";
            }
            "tSPACE" => {
                let leading = if value.starts_with(b"\n") {
                    1
                } else {
                    value.iter().take_while(|&&b| b != b'\n').count()
                };
                end = start + leading;
            }
            "tSTRING_BEG" => {
                let next = lexed.get(index);
                let next_next = lexed.get(index + 1);
                let basic = matches!(value, b"\"" | b"'");
                if basic && next.is_some_and(|t| t.kind == sys::PM_TOKEN_STRING_END) {
                    kind = "tSTRING";
                    end = next.unwrap().end;
                    index += 1;
                } else if value.starts_with(b"'")
                    || value.starts_with(b"\"")
                    || value.starts_with(b"%")
                {
                    if let (Some(content), Some(closing)) = (next, next_next)
                        && content.kind == sys::PM_TOKEN_STRING_CONTENT
                        && closing.kind == sys::PM_TOKEN_STRING_END
                        && simplify(&bytes[content.start..content.end], value)
                    {
                        tokens.push(("tSTRING", start, closing.end));
                        index += 2;
                        continue;
                    }
                    quotes.push(value);
                } else if token.kind == sys::PM_TOKEN_HEREDOC_START {
                    let raw = value.get(2..).unwrap_or_default();
                    let raw = raw
                        .strip_prefix(b"-")
                        .or(raw.strip_prefix(b"~"))
                        .unwrap_or(raw);
                    let quote: &[u8] = match raw.first() {
                        Some(b'`') => {
                            kind = "tXSTRING_BEG";
                            b"<<`"
                        }
                        Some(b'\'') => b"<<'",
                        _ => b"<<\"",
                    };
                    quotes.push(quote);
                }
            }
            "tSTRING_CONTENT" => {
                let quote = quotes.last().copied();
                let lines: Vec<&[u8]> = value.split_inclusive(|&b| b == b'\n').collect();
                if lines.len() <= 1 {
                    // A single line; heredoc lines joined by continuations.
                    let begin = start;
                    let mut length = 0;
                    let mut current = token;
                    loop {
                        length += current.end - current.start;
                        let text = &bytes[current.start..current.end];
                        let backslashes = text
                            .strip_suffix(b"\n")
                            .map_or(0, |b| b.iter().rev().take_while(|&&c| c == b'\\').count());
                        let backslashes = if quote
                            .is_some_and(|q| q.starts_with(b"%W") || q.starts_with(b"%I"))
                        {
                            0
                        } else {
                            backslashes
                        };
                        if backslashes % 2 == 0 || !interpolation(quote) {
                            tokens.push(("tSTRING_CONTENT", begin, begin + length));
                            break;
                        }
                        let Some(&following) = lexed.get(index) else {
                            break;
                        };
                        index += 1;
                        if following.kind != sys::PM_TOKEN_STRING_CONTENT {
                            break;
                        }
                        current = following;
                    }
                } else {
                    let mut piece = start;
                    let mut cursor = start;
                    let count = lines.len();
                    for (i, line) in lines.into_iter().enumerate() {
                        cursor += line.len();
                        let body = line.strip_suffix(b"\n").unwrap_or(line);
                        let body = body.strip_suffix(b"\r").unwrap_or(body);
                        let odd = body.iter().rev().take_while(|&&b| b == b'\\').count() % 2 == 1;
                        let joined = odd && (interpolation(quote) || percent_array(quote));
                        if !joined || i == count - 1 {
                            tokens.push(("tSTRING_CONTENT", piece, cursor));
                            piece = cursor;
                        }
                    }
                }
                continue;
            }
            "tSTRING_END" => {
                if token.kind == sys::PM_TOKEN_HEREDOC_END && value.ends_with(b"\n") {
                    end -= if value.ends_with(b"\r\n") { 2 } else { 1 };
                } else if token.kind == sys::PM_TOKEN_REGEXP_END {
                    end = start + 1;
                }
                if percent_array(quotes.pop()) {
                    let previous = index.checked_sub(2).and_then(|i| lexed.get(i));
                    let empty = previous.is_some_and(|p| {
                        matches!(
                            p.kind,
                            sys::PM_TOKEN_PERCENT_LOWER_I
                                | sys::PM_TOKEN_PERCENT_LOWER_W
                                | sys::PM_TOKEN_PERCENT_UPPER_I
                                | sys::PM_TOKEN_PERCENT_UPPER_W
                        )
                    });
                    let separated = previous.is_some_and(|p| p.kind == sys::PM_TOKEN_WORDS_SEP);
                    if !empty && !separated {
                        tokens.push(("tSPACE", start, start));
                    }
                }
            }
            "tSYMBEG" => {
                if let Some(next) = lexed.get(index)
                    && !matches!(
                        next.kind,
                        sys::PM_TOKEN_STRING_CONTENT
                            | sys::PM_TOKEN_EMBEXPR_BEGIN
                            | sys::PM_TOKEN_EMBVAR
                            | sys::PM_TOKEN_STRING_END
                    )
                {
                    kind = "tSYMBOL";
                    end = next.end;
                    index += 1;
                } else {
                    quotes.push(value);
                }
            }
            "tFID" if tokens.last().is_some_and(|t| t.0 == "kDEF") => kind = "tIDENTIFIER",
            "tXSTRING_BEG" => {
                if lexed.get(index).is_some_and(|next| {
                    !matches!(
                        next.kind,
                        sys::PM_TOKEN_STRING_CONTENT
                            | sys::PM_TOKEN_STRING_END
                            | sys::PM_TOKEN_EMBEXPR_BEGIN
                    )
                }) {
                    kind = "tBACK_REF2";
                }
                quotes.push(value);
            }
            "tSYMBOLS_BEG" | "tQSYMBOLS_BEG" | "tWORDS_BEG" | "tQWORDS_BEG" => {
                if kind_at(index) == Some(sys::PM_TOKEN_WORDS_SEP) {
                    index += 1;
                }
                quotes.push(value);
            }
            "tREGEXP_BEG" => quotes.push(value),
            _ => {}
        }
        tokens.push((kind, start, end));
        if token.kind == sys::PM_TOKEN_REGEXP_END {
            tokens.push((
                "tREGEXP_OPT",
                token.start + 1,
                token.end.max(token.start + 1),
            ));
        }
    }
    tokens
}

/// The translator's `TYPES` table.
fn parser_type(kind: sys::pm_token_type) -> Option<&'static str> {
    Some(match kind {
        sys::PM_TOKEN_AMPERSAND => "tAMPER2",
        sys::PM_TOKEN_AMPERSAND_AMPERSAND => "tANDOP",
        sys::PM_TOKEN_AMPERSAND_AMPERSAND_EQUAL => "tOP_ASGN",
        sys::PM_TOKEN_AMPERSAND_DOT => "tANDDOT",
        sys::PM_TOKEN_AMPERSAND_EQUAL => "tOP_ASGN",
        sys::PM_TOKEN_BACK_REFERENCE => "tBACK_REF",
        sys::PM_TOKEN_BACKTICK => "tXSTRING_BEG",
        sys::PM_TOKEN_BANG => "tBANG",
        sys::PM_TOKEN_BANG_EQUAL => "tNEQ",
        sys::PM_TOKEN_BANG_TILDE => "tNMATCH",
        sys::PM_TOKEN_BRACE_LEFT => "tLCURLY",
        sys::PM_TOKEN_BRACE_RIGHT => "tRCURLY",
        sys::PM_TOKEN_BRACKET_LEFT => "tLBRACK2",
        sys::PM_TOKEN_BRACKET_LEFT_ARRAY => "tLBRACK",
        sys::PM_TOKEN_BRACKET_LEFT_RIGHT => "tAREF",
        sys::PM_TOKEN_BRACKET_LEFT_RIGHT_EQUAL => "tASET",
        sys::PM_TOKEN_BRACKET_RIGHT => "tRBRACK",
        sys::PM_TOKEN_CARET => "tCARET",
        sys::PM_TOKEN_CARET_EQUAL => "tOP_ASGN",
        sys::PM_TOKEN_CHARACTER_LITERAL => "tCHARACTER",
        sys::PM_TOKEN_CLASS_VARIABLE => "tCVAR",
        sys::PM_TOKEN_COLON => "tCOLON",
        sys::PM_TOKEN_COLON_COLON => "tCOLON2",
        sys::PM_TOKEN_COMMA => "tCOMMA",
        sys::PM_TOKEN_COMMENT => "tCOMMENT",
        sys::PM_TOKEN_CONSTANT => "tCONSTANT",
        sys::PM_TOKEN_DOT => "tDOT",
        sys::PM_TOKEN_DOT_DOT => "tDOT2",
        sys::PM_TOKEN_DOT_DOT_DOT => "tDOT3",
        sys::PM_TOKEN_EMBDOC_BEGIN => "tCOMMENT",
        sys::PM_TOKEN_EMBEXPR_BEGIN => "tSTRING_DBEG",
        sys::PM_TOKEN_EMBEXPR_END => "tSTRING_DEND",
        sys::PM_TOKEN_EMBVAR => "tSTRING_DVAR",
        sys::PM_TOKEN_EQUAL => "tEQL",
        sys::PM_TOKEN_EQUAL_EQUAL => "tEQ",
        sys::PM_TOKEN_EQUAL_EQUAL_EQUAL => "tEQQ",
        sys::PM_TOKEN_EQUAL_GREATER => "tASSOC",
        sys::PM_TOKEN_EQUAL_TILDE => "tMATCH",
        sys::PM_TOKEN_FLOAT => "tFLOAT",
        sys::PM_TOKEN_FLOAT_IMAGINARY => "tIMAGINARY",
        sys::PM_TOKEN_FLOAT_RATIONAL => "tRATIONAL",
        sys::PM_TOKEN_FLOAT_RATIONAL_IMAGINARY => "tIMAGINARY",
        sys::PM_TOKEN_GLOBAL_VARIABLE => "tGVAR",
        sys::PM_TOKEN_GREATER => "tGT",
        sys::PM_TOKEN_GREATER_EQUAL => "tGEQ",
        sys::PM_TOKEN_GREATER_GREATER => "tRSHFT",
        sys::PM_TOKEN_GREATER_GREATER_EQUAL => "tOP_ASGN",
        sys::PM_TOKEN_HEREDOC_START => "tSTRING_BEG",
        sys::PM_TOKEN_HEREDOC_END => "tSTRING_END",
        sys::PM_TOKEN_IDENTIFIER => "tIDENTIFIER",
        sys::PM_TOKEN_INSTANCE_VARIABLE => "tIVAR",
        sys::PM_TOKEN_INTEGER => "tINTEGER",
        sys::PM_TOKEN_INTEGER_IMAGINARY => "tIMAGINARY",
        sys::PM_TOKEN_INTEGER_RATIONAL => "tRATIONAL",
        sys::PM_TOKEN_INTEGER_RATIONAL_IMAGINARY => "tIMAGINARY",
        sys::PM_TOKEN_KEYWORD_ALIAS => "kALIAS",
        sys::PM_TOKEN_KEYWORD_AND => "kAND",
        sys::PM_TOKEN_KEYWORD_BEGIN => "kBEGIN",
        sys::PM_TOKEN_KEYWORD_BEGIN_UPCASE => "klBEGIN",
        sys::PM_TOKEN_KEYWORD_BREAK => "kBREAK",
        sys::PM_TOKEN_KEYWORD_CASE => "kCASE",
        sys::PM_TOKEN_KEYWORD_CLASS => "kCLASS",
        sys::PM_TOKEN_KEYWORD_DEF => "kDEF",
        sys::PM_TOKEN_KEYWORD_DEFINED => "kDEFINED",
        sys::PM_TOKEN_KEYWORD_DO => "kDO",
        sys::PM_TOKEN_KEYWORD_DO_LOOP => "kDO_COND",
        sys::PM_TOKEN_KEYWORD_END => "kEND",
        sys::PM_TOKEN_KEYWORD_END_UPCASE => "klEND",
        sys::PM_TOKEN_KEYWORD_ENSURE => "kENSURE",
        sys::PM_TOKEN_KEYWORD_ELSE => "kELSE",
        sys::PM_TOKEN_KEYWORD_ELSIF => "kELSIF",
        sys::PM_TOKEN_KEYWORD_FALSE => "kFALSE",
        sys::PM_TOKEN_KEYWORD_FOR => "kFOR",
        sys::PM_TOKEN_KEYWORD_IF => "kIF",
        sys::PM_TOKEN_KEYWORD_IF_MODIFIER => "kIF_MOD",
        sys::PM_TOKEN_KEYWORD_IN => "kIN",
        sys::PM_TOKEN_KEYWORD_MODULE => "kMODULE",
        sys::PM_TOKEN_KEYWORD_NEXT => "kNEXT",
        sys::PM_TOKEN_KEYWORD_NIL => "kNIL",
        sys::PM_TOKEN_KEYWORD_NOT => "kNOT",
        sys::PM_TOKEN_KEYWORD_OR => "kOR",
        sys::PM_TOKEN_KEYWORD_REDO => "kREDO",
        sys::PM_TOKEN_KEYWORD_RESCUE => "kRESCUE",
        sys::PM_TOKEN_KEYWORD_RESCUE_MODIFIER => "kRESCUE_MOD",
        sys::PM_TOKEN_KEYWORD_RETRY => "kRETRY",
        sys::PM_TOKEN_KEYWORD_RETURN => "kRETURN",
        sys::PM_TOKEN_KEYWORD_SELF => "kSELF",
        sys::PM_TOKEN_KEYWORD_SUPER => "kSUPER",
        sys::PM_TOKEN_KEYWORD_THEN => "kTHEN",
        sys::PM_TOKEN_KEYWORD_TRUE => "kTRUE",
        sys::PM_TOKEN_KEYWORD_UNDEF => "kUNDEF",
        sys::PM_TOKEN_KEYWORD_UNLESS => "kUNLESS",
        sys::PM_TOKEN_KEYWORD_UNLESS_MODIFIER => "kUNLESS_MOD",
        sys::PM_TOKEN_KEYWORD_UNTIL => "kUNTIL",
        sys::PM_TOKEN_KEYWORD_UNTIL_MODIFIER => "kUNTIL_MOD",
        sys::PM_TOKEN_KEYWORD_WHEN => "kWHEN",
        sys::PM_TOKEN_KEYWORD_WHILE => "kWHILE",
        sys::PM_TOKEN_KEYWORD_WHILE_MODIFIER => "kWHILE_MOD",
        sys::PM_TOKEN_KEYWORD_YIELD => "kYIELD",
        sys::PM_TOKEN_KEYWORD___ENCODING__ => "k__ENCODING__",
        sys::PM_TOKEN_KEYWORD___FILE__ => "k__FILE__",
        sys::PM_TOKEN_KEYWORD___LINE__ => "k__LINE__",
        sys::PM_TOKEN_LABEL => "tLABEL",
        sys::PM_TOKEN_LABEL_END => "tLABEL_END",
        sys::PM_TOKEN_LAMBDA_BEGIN => "tLAMBEG",
        sys::PM_TOKEN_LESS => "tLT",
        sys::PM_TOKEN_LESS_EQUAL => "tLEQ",
        sys::PM_TOKEN_LESS_EQUAL_GREATER => "tCMP",
        sys::PM_TOKEN_LESS_LESS => "tLSHFT",
        sys::PM_TOKEN_LESS_LESS_EQUAL => "tOP_ASGN",
        sys::PM_TOKEN_METHOD_NAME => "tFID",
        sys::PM_TOKEN_MINUS => "tMINUS",
        sys::PM_TOKEN_MINUS_EQUAL => "tOP_ASGN",
        sys::PM_TOKEN_MINUS_GREATER => "tLAMBDA",
        sys::PM_TOKEN_NEWLINE => "tNL",
        sys::PM_TOKEN_NUMBERED_REFERENCE => "tNTH_REF",
        sys::PM_TOKEN_PARENTHESIS_LEFT => "tLPAREN2",
        sys::PM_TOKEN_PARENTHESIS_LEFT_PARENTHESES => "tLPAREN_ARG",
        sys::PM_TOKEN_PARENTHESIS_RIGHT => "tRPAREN",
        sys::PM_TOKEN_PERCENT => "tPERCENT",
        sys::PM_TOKEN_PERCENT_EQUAL => "tOP_ASGN",
        sys::PM_TOKEN_PERCENT_LOWER_I => "tQSYMBOLS_BEG",
        sys::PM_TOKEN_PERCENT_LOWER_W => "tQWORDS_BEG",
        sys::PM_TOKEN_PERCENT_UPPER_I => "tSYMBOLS_BEG",
        sys::PM_TOKEN_PERCENT_UPPER_W => "tWORDS_BEG",
        sys::PM_TOKEN_PERCENT_LOWER_X => "tXSTRING_BEG",
        sys::PM_TOKEN_PLUS => "tPLUS",
        sys::PM_TOKEN_PLUS_EQUAL => "tOP_ASGN",
        sys::PM_TOKEN_PIPE_EQUAL => "tOP_ASGN",
        sys::PM_TOKEN_PIPE => "tPIPE",
        sys::PM_TOKEN_PIPE_PIPE => "tOROP",
        sys::PM_TOKEN_PIPE_PIPE_EQUAL => "tOP_ASGN",
        sys::PM_TOKEN_QUESTION_MARK => "tEH",
        sys::PM_TOKEN_REGEXP_BEGIN => "tREGEXP_BEG",
        sys::PM_TOKEN_REGEXP_END => "tSTRING_END",
        sys::PM_TOKEN_SEMICOLON => "tSEMI",
        sys::PM_TOKEN_SLASH => "tDIVIDE",
        sys::PM_TOKEN_SLASH_EQUAL => "tOP_ASGN",
        sys::PM_TOKEN_STAR => "tSTAR2",
        sys::PM_TOKEN_STAR_EQUAL => "tOP_ASGN",
        sys::PM_TOKEN_STAR_STAR => "tPOW",
        sys::PM_TOKEN_STAR_STAR_EQUAL => "tOP_ASGN",
        sys::PM_TOKEN_STRING_BEGIN => "tSTRING_BEG",
        sys::PM_TOKEN_STRING_CONTENT => "tSTRING_CONTENT",
        sys::PM_TOKEN_STRING_END => "tSTRING_END",
        sys::PM_TOKEN_SYMBOL_BEGIN => "tSYMBEG",
        sys::PM_TOKEN_TILDE => "tTILDE",
        sys::PM_TOKEN_UAMPERSAND => "tAMPER",
        sys::PM_TOKEN_UCOLON_COLON => "tCOLON3",
        sys::PM_TOKEN_UDOT_DOT => "tBDOT2",
        sys::PM_TOKEN_UDOT_DOT_DOT => "tBDOT3",
        sys::PM_TOKEN_UMINUS => "tUMINUS",
        sys::PM_TOKEN_UMINUS_NUM => "tUNARY_NUM",
        sys::PM_TOKEN_UPLUS => "tUPLUS",
        sys::PM_TOKEN_USTAR => "tSTAR",
        sys::PM_TOKEN_USTAR_STAR => "tDSTAR",
        sys::PM_TOKEN_WORDS_SEP => "tSPACE",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(source: &str) -> Vec<(&'static str, &str)> {
        let file = SourceFile::new("t.rb".into(), source.as_bytes().to_vec());
        collect(&file)
            .into_iter()
            .map(|t| (t.kind, &source[t.start..t.end]))
            .collect()
    }

    #[test]
    fn strings_symbols_and_regexps_merge_or_split_like_the_translator() {
        assert_eq!(
            tokens("x = 'a  b' + \"\" + :sym + /re/i\n"),
            [
                ("tIDENTIFIER", "x"),
                ("tEQL", "="),
                ("tSTRING", "'a  b'"),
                ("tPLUS", "+"),
                ("tSTRING", "\"\""),
                ("tPLUS", "+"),
                ("tSYMBOL", ":sym"),
                ("tPLUS", "+"),
                ("tREGEXP_BEG", "/"),
                ("tSTRING_CONTENT", "re"),
                ("tSTRING_END", "/"),
                ("tREGEXP_OPT", "i"),
                ("tNL", "\n"),
            ]
        );
        assert_eq!(
            tokens("%w[a  b]\n"),
            [
                ("tQWORDS_BEG", "%w["),
                ("tSTRING_CONTENT", "a"),
                ("tSPACE", "  "),
                ("tSTRING_CONTENT", "b"),
                ("tSPACE", ""),
                ("tSTRING_END", "]"),
                ("tNL", "\n"),
            ]
        );
        assert_eq!(
            tokens("\"a\nb\" + 'c\nd'\n"),
            [
                ("tSTRING_BEG", "\""),
                ("tSTRING_CONTENT", "a\n"),
                ("tSTRING_CONTENT", "b"),
                ("tSTRING_END", "\""),
                ("tPLUS", "+"),
                ("tSTRING_BEG", "'"),
                ("tSTRING_CONTENT", "c\n"),
                ("tSTRING_CONTENT", "d"),
                ("tSTRING_END", "'"),
                ("tNL", "\n"),
            ]
        );
    }

    #[test]
    fn comments_newlines_and_heredocs_follow_the_translator() {
        assert_eq!(
            tokens("a  # c\n# d\nfoo(<<~EOS, 1)\n  x\nEOS\n"),
            [
                ("tIDENTIFIER", "a"),
                // The inline comment's newline follows the next comment.
                ("tCOMMENT", "# c"),
                ("tCOMMENT", "# d"),
                ("tNL", "\n"),
                ("tIDENTIFIER", "foo"),
                ("tLPAREN2", "("),
                ("tSTRING_BEG", "<<~EOS"),
                ("tSTRING_CONTENT", "  x\n"),
                ("tSTRING_END", "EOS"),
                ("tCOMMA", ","),
                ("tINTEGER", "1"),
                ("tRPAREN", ")"),
                ("tNL", "\n"),
            ]
        );
        assert_eq!(
            tokens("=begin\nx\n=end\nf { |a| a }\n"),
            [
                ("tCOMMENT", "=begin\nx\n=end\n"),
                ("tIDENTIFIER", "f"),
                ("tLCURLY", "{"),
                ("tPIPE", "|"),
                ("tIDENTIFIER", "a"),
                ("tPIPE", "|"),
                ("tIDENTIFIER", "a"),
                ("tRCURLY", "}"),
                ("tNL", "\n"),
            ]
        );
        assert_eq!(
            tokens("x = { a: +1 }\n__END__\ndata  here\n"),
            [
                ("tIDENTIFIER", "x"),
                ("tEQL", "="),
                ("tLBRACE", "{"),
                ("tLABEL", "a:"),
                ("tUNARY_NUM", "+"),
                ("tINTEGER", "1"),
                ("tRCURLY", "}"),
                ("tNL", "\n"),
            ]
        );
    }

    #[test]
    fn positions_are_character_columns_on_the_start_line() {
        let file = SourceFile::new("t.rb".into(), "\u{feff}é = 1\r\nab  = 2\n".into());
        let found = collect(&file);
        let at = |text: &str, line| {
            found
                .iter()
                .find(|t| &file.bytes[t.start..t.end] == text.as_bytes() && t.line == line)
                .map(|t| (t.column, t.end_column))
                .unwrap()
        };
        assert_eq!(at("é", 1), (0, 1));
        assert_eq!(at("=", 1), (2, 3));
        assert_eq!(at("=", 2), (4, 5));
    }
}
