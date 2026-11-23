//! Ultra-fast single-pass Rust syntax highlighter for Markdown code blocks.
//! Produces `<span class="hl-...">` HTML without regex backtracking.

pub fn escape_html(s: &str, out: &mut String) {
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
}

pub fn escape_html_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    escape_html(s, &mut out);
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lang {
    Rust,
    Python,
    JsTs,
    CLike,
    Go,
    Shell,
    Sql,
    Json,
    TomlYaml,
    HtmlCss,
    Latex,
    Generic,
    Plain,
}

fn detect_lang(lang: &str) -> Lang {
    match lang.trim().to_ascii_lowercase().as_str() {
        "rs" | "rust" => Lang::Rust,
        "py" | "python" | "py3" => Lang::Python,
        "js" | "jsx" | "ts" | "tsx" | "javascript" | "typescript" | "mjs" | "cjs" => Lang::JsTs,
        "c" | "h" | "cpp" | "cc" | "cxx" | "hpp" | "java" | "cs" | "csharp" | "swift"
        | "kotlin" | "kt" | "zig" | "dart" => Lang::CLike,
        "go" | "golang" => Lang::Go,
        "sh" | "bash" | "zsh" | "shell" | "ps1" | "powershell" | "fish" => Lang::Shell,
        "sql" | "postgres" | "mysql" | "sqlite" => Lang::Sql,
        "json" | "jsonc" => Lang::Json,
        "toml" | "yaml" | "yml" | "ini" => Lang::TomlYaml,
        "html" | "xml" | "svg" | "css" | "scss" | "less" => Lang::HtmlCss,
        "tex" | "latex" | "math" => Lang::Latex,
        "text" | "txt" | "plain" | "plaintext" => Lang::Plain,
        _ => Lang::Generic,
    }
}

fn is_keyword(lang: Lang, word: &str) -> bool {
    match lang {
        Lang::Generic => [Lang::Rust, Lang::Python, Lang::JsTs]
            .iter()
            .any(|&language| is_keyword(language, word)),
        Lang::Rust => matches!(
            word,
            "fn" | "let"
                | "mut"
                | "pub"
                | "struct"
                | "enum"
                | "impl"
                | "trait"
                | "for"
                | "in"
                | "while"
                | "loop"
                | "if"
                | "else"
                | "match"
                | "return"
                | "use"
                | "mod"
                | "crate"
                | "super"
                | "self"
                | "Self"
                | "where"
                | "async"
                | "await"
                | "move"
                | "ref"
                | "const"
                | "static"
                | "type"
                | "unsafe"
                | "extern"
                | "as"
                | "break"
                | "continue"
                | "true"
                | "false"
                | "dyn"
        ),
        Lang::Python => matches!(
            word,
            "def"
                | "class"
                | "if"
                | "elif"
                | "else"
                | "for"
                | "while"
                | "in"
                | "not"
                | "and"
                | "or"
                | "is"
                | "return"
                | "yield"
                | "import"
                | "from"
                | "as"
                | "with"
                | "try"
                | "except"
                | "finally"
                | "raise"
                | "pass"
                | "break"
                | "continue"
                | "lambda"
                | "async"
                | "await"
                | "True"
                | "False"
                | "None"
                | "self"
                | "global"
                | "nonlocal"
                | "assert"
                | "del"
        ),
        Lang::JsTs => matches!(
            word,
            "function"
                | "const"
                | "let"
                | "var"
                | "if"
                | "else"
                | "for"
                | "while"
                | "do"
                | "switch"
                | "case"
                | "default"
                | "break"
                | "continue"
                | "return"
                | "class"
                | "extends"
                | "new"
                | "this"
                | "super"
                | "import"
                | "export"
                | "from"
                | "async"
                | "await"
                | "try"
                | "catch"
                | "finally"
                | "throw"
                | "typeof"
                | "instanceof"
                | "in"
                | "of"
                | "true"
                | "false"
                | "null"
                | "undefined"
                | "interface"
                | "type"
                | "enum"
                | "implements"
                | "private"
                | "public"
                | "protected"
                | "readonly"
                | "as"
        ),
        Lang::CLike => matches!(
            word,
            "int"
                | "long"
                | "short"
                | "char"
                | "float"
                | "double"
                | "void"
                | "bool"
                | "auto"
                | "const"
                | "static"
                | "struct"
                | "class"
                | "enum"
                | "union"
                | "typedef"
                | "using"
                | "namespace"
                | "template"
                | "typename"
                | "public"
                | "private"
                | "protected"
                | "virtual"
                | "override"
                | "if"
                | "else"
                | "for"
                | "while"
                | "do"
                | "switch"
                | "case"
                | "default"
                | "break"
                | "continue"
                | "return"
                | "new"
                | "delete"
                | "sizeof"
                | "true"
                | "false"
                | "nullptr"
                | "NULL"
                | "try"
                | "catch"
                | "throw"
                | "inline"
                | "constexpr"
                | "final"
                | "package"
                | "import"
                | "extends"
                | "implements"
        ),
        Lang::Go => matches!(
            word,
            "func"
                | "package"
                | "import"
                | "var"
                | "const"
                | "type"
                | "struct"
                | "interface"
                | "map"
                | "chan"
                | "if"
                | "else"
                | "for"
                | "range"
                | "switch"
                | "case"
                | "default"
                | "select"
                | "go"
                | "defer"
                | "return"
                | "break"
                | "continue"
                | "true"
                | "false"
                | "nil"
        ),
        Lang::Shell => matches!(
            word,
            "if" | "then"
                | "else"
                | "elif"
                | "fi"
                | "for"
                | "while"
                | "do"
                | "done"
                | "case"
                | "esac"
                | "in"
                | "function"
                | "return"
                | "export"
                | "local"
                | "echo"
                | "cd"
                | "set"
                | "exit"
                | "true"
                | "false"
        ),
        Lang::Sql => {
            let up = word.to_ascii_uppercase();
            matches!(
                up.as_str(),
                "SELECT"
                    | "FROM"
                    | "WHERE"
                    | "JOIN"
                    | "LEFT"
                    | "RIGHT"
                    | "INNER"
                    | "OUTER"
                    | "ON"
                    | "GROUP"
                    | "BY"
                    | "ORDER"
                    | "HAVING"
                    | "LIMIT"
                    | "OFFSET"
                    | "INSERT"
                    | "INTO"
                    | "VALUES"
                    | "UPDATE"
                    | "SET"
                    | "DELETE"
                    | "CREATE"
                    | "TABLE"
                    | "INDEX"
                    | "DROP"
                    | "ALTER"
                    | "AS"
                    | "AND"
                    | "OR"
                    | "NOT"
                    | "NULL"
                    | "IS"
                    | "IN"
                    | "DISTINCT"
                    | "COUNT"
                    | "SUM"
                    | "AVG"
                    | "PRIMARY"
                    | "KEY"
                    | "FOREIGN"
                    | "REFERENCES"
                    | "WITH"
                    | "UNION"
                    | "CASE"
                    | "WHEN"
                    | "THEN"
                    | "ELSE"
                    | "END"
            )
        }
        Lang::Json | Lang::TomlYaml => matches!(word, "true" | "false" | "null" | "yes" | "no"),
        _ => false,
    }
}

fn is_builtin_type(lang: Lang, word: &str) -> bool {
    match lang {
        Lang::Rust => matches!(
            word,
            "u8" | "u16"
                | "u32"
                | "u64"
                | "u128"
                | "usize"
                | "i8"
                | "i16"
                | "i32"
                | "i64"
                | "i128"
                | "isize"
                | "f32"
                | "f64"
                | "bool"
                | "char"
                | "str"
                | "String"
                | "Vec"
                | "Option"
                | "Result"
                | "Box"
                | "Arc"
                | "Rc"
                | "HashMap"
                | "HashSet"
                | "Path"
                | "PathBuf"
                | "Some"
                | "None"
                | "Ok"
                | "Err"
        ),
        Lang::Go => matches!(
            word,
            "int"
                | "int64"
                | "int32"
                | "uint"
                | "uint64"
                | "float64"
                | "float32"
                | "string"
                | "bool"
                | "byte"
                | "rune"
                | "error"
                | "any"
        ),
        Lang::JsTs => matches!(
            word,
            "string"
                | "number"
                | "boolean"
                | "void"
                | "any"
                | "unknown"
                | "never"
                | "Promise"
                | "Array"
                | "Map"
                | "Set"
                | "Record"
                | "Object"
        ),
        Lang::Python => matches!(
            word,
            "int"
                | "float"
                | "str"
                | "bool"
                | "list"
                | "dict"
                | "set"
                | "tuple"
                | "bytes"
                | "object"
                | "print"
                | "len"
                | "range"
                | "enumerate"
                | "zip"
        ),
        _ => false,
    }
}

fn push_span(out: &mut String, cls: &str, text: &str) {
    out.push_str("<span class=\"");
    out.push_str(cls);
    out.push_str("\">");
    escape_html(text, out);
    out.push_str("</span>");
}

pub fn highlight_code(code: &str, lang_name: &str) -> String {
    let lang = detect_lang(lang_name);
    if lang == Lang::Plain {
        return escape_html_string(code);
    }

    let mut out = String::with_capacity(code.len() + code.len() / 2);
    let bytes = code.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        let b = bytes[i];

        // Line comments
        if matches!(
            lang,
            Lang::Rust | Lang::JsTs | Lang::CLike | Lang::Go | Lang::HtmlCss | Lang::Generic
        ) && b == b'/'
            && i + 1 < len
            && bytes[i + 1] == b'/'
        {
            let start = i;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            push_span(&mut out, "hl-cmt", &code[start..i]);
            continue;
        }

        // Block comments /* ... */
        if matches!(
            lang,
            Lang::Rust
                | Lang::JsTs
                | Lang::CLike
                | Lang::Go
                | Lang::HtmlCss
                | Lang::Sql
                | Lang::Generic
        ) && b == b'/'
            && i + 1 < len
            && bytes[i + 1] == b'*'
        {
            let start = i;
            i += 2;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(len);
            push_span(&mut out, "hl-cmt", &code[start..i]);
            continue;
        }

        // Hash comments (#) for Python, Shell, TOML/YAML
        if matches!(
            lang,
            Lang::Python | Lang::Shell | Lang::TomlYaml | Lang::Generic
        ) && b == b'#'
        {
            let start = i;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            push_span(&mut out, "hl-cmt", &code[start..i]);
            continue;
        }

        // SQL -- comments
        if lang == Lang::Sql && b == b'-' && i + 1 < len && bytes[i + 1] == b'-' {
            let start = i;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            push_span(&mut out, "hl-cmt", &code[start..i]);
            continue;
        }

        // LaTeX % comments & \commands
        if lang == Lang::Latex {
            if b == b'%' {
                let start = i;
                while i < len && bytes[i] != b'\n' {
                    i += 1;
                }
                push_span(&mut out, "hl-cmt", &code[start..i]);
                continue;
            }
            if b == b'\\' {
                let start = i;
                i += 1;
                if i < len && bytes[i].is_ascii_alphabetic() {
                    while i < len && bytes[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                } else if i < len {
                    i += code[i..].chars().next().unwrap().len_utf8();
                }
                push_span(&mut out, "hl-kw", &code[start..i]);
                continue;
            }
        }

        // C preprocessor / Rust attributes (#[...])
        if (lang == Lang::CLike || lang == Lang::Rust) && b == b'#' {
            let start = i;
            if lang == Lang::CLike {
                while i < len && bytes[i] != b'\n' {
                    i += 1;
                }
            } else {
                i += 1;
                if i < len && bytes[i] == b'!' {
                    i += 1;
                }
                if i < len && bytes[i] == b'[' {
                    let mut depth = 1usize;
                    i += 1;
                    while i < len && depth > 0 {
                        if bytes[i] == b'[' {
                            depth += 1;
                        } else if bytes[i] == b']' {
                            depth -= 1;
                        }
                        i += 1;
                    }
                }
            }
            push_span(&mut out, "hl-mac", &code[start..i]);
            continue;
        }

        // Strings ("...", '...', `...`)
        if b == b'"'
            || (b == b'\'' && lang != Lang::Rust)
            || (b == b'`' && matches!(lang, Lang::JsTs | Lang::Go | Lang::Shell))
        {
            let quote = b;
            let start = i;
            i += 1;
            while i < len {
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 2;
                    continue;
                }
                if bytes[i] == quote {
                    i += 1;
                    break;
                }
                if bytes[i] == b'\n' && quote != b'`' {
                    break;
                }
                i += 1;
            }
            // In JSON, distinguish property keys from string values
            if lang == Lang::Json {
                let mut k = i;
                while k < len && (bytes[k] == b' ' || bytes[k] == b'\t') {
                    k += 1;
                }
                if k < len && bytes[k] == b':' {
                    push_span(&mut out, "hl-key", &code[start..i]);
                    continue;
                }
            }
            push_span(&mut out, "hl-str", &code[start..i]);
            continue;
        }

        // Numbers
        if b.is_ascii_digit()
            && (i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_'))
        {
            let start = i;
            i += 1;
            while i < len
                && (bytes[i].is_ascii_alphanumeric()
                    || bytes[i] == b'.'
                    || bytes[i] == b'_'
                    || bytes[i] == b'x'
                    || bytes[i] == b'X')
            {
                i += 1;
            }
            push_span(&mut out, "hl-num", &code[start..i]);
            continue;
        }

        // Identifiers / Keywords / Types / Function calls / Rust macros
        if b.is_ascii_alphabetic() || b == b'_' {
            let start = i;
            i += 1;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &code[start..i];
            if lang == Lang::Rust && i < len && bytes[i] == b'!' {
                i += 1;
                push_span(&mut out, "hl-mac", &code[start..i]);
            } else if is_keyword(lang, word) {
                push_span(&mut out, "hl-kw", word);
            } else if is_builtin_type(lang, word)
                || (word.len() > 1
                    && word.as_bytes()[0].is_ascii_uppercase()
                    && word.as_bytes().iter().any(|c| c.is_ascii_lowercase()))
            {
                push_span(&mut out, "hl-ty", word);
            } else if i < len && bytes[i] == b'(' {
                push_span(&mut out, "hl-fn", word);
            } else {
                escape_html(word, &mut out);
            }
            continue;
        }

        // UTF-8 safe single character copy
        let ch = code[i..].chars().next().unwrap();
        let ch_len = ch.len_utf8();
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
        i += ch_len;
    }

    out
}
