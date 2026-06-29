//! Minimal **inline** markdown for chat bodies, matching Keybase's set:
//! `*bold*`, `_italic_`, `~strike~`, `` `code` ``. Emphasis markers must hug
//! non-space (so `2*3` is not bold) and are hidden when rendered; `` `code` ``
//! is verbatim (no inner markup). `@mentions` are highlighted in the same pass.
//!
//! Pure (no view types): produces styled [`Run`]s; the view maps the flags to
//! ratatui styles and wraps them to the panel width.

/// A styled fragment of a message line.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Run {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub mention: bool,
}

/// Parses one line into styled runs (emphasis + code + mentions).
pub fn parse_inline(line: &str, mentions: &[String]) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    let mut plain = 0usize;
    let mut i = 0usize;
    while i < line.len() {
        let c = line[i..].chars().next().unwrap();
        let span = match c {
            '`' => code_span(line, i),
            '*' | '_' | '~' => emph_span(line, i, c),
            _ => None,
        };
        if let Some((inner_start, inner_end, end, base)) = span {
            push_mentions(&mut out, &line[plain..i], &Run::default(), mentions);
            if base.code {
                // Code is verbatim — no inner markup or mention highlighting.
                out.push(Run {
                    text: line[inner_start..inner_end].to_string(),
                    code: true,
                    ..Run::default()
                });
            } else {
                push_mentions(&mut out, &line[inner_start..inner_end], &base, mentions);
            }
            i = end;
            plain = end;
        } else {
            i += c.len_utf8();
        }
    }
    push_mentions(&mut out, &line[plain..], &Run::default(), mentions);
    out
}

/// `` `code` `` — paired backticks; returns (inner start, inner end, end, base).
fn code_span(line: &str, at: usize) -> Option<(usize, usize, usize, Run)> {
    let close = line[at + 1..].find('`')? + at + 1;
    if close == at + 1 {
        return None; // empty ``
    }
    Some((
        at + 1,
        close,
        close + 1,
        Run {
            code: true,
            ..Run::default()
        },
    ))
}

/// `*bold*` / `_italic_` / `~strike~` with flanking rules.
fn emph_span(line: &str, at: usize, marker: char) -> Option<(usize, usize, usize, Run)> {
    // Opener: at a boundary (start / whitespace / punctuation) and hugging a
    // non-space, non-marker char.
    if !boundary_before(line, at) {
        return None;
    }
    let first = line[at + 1..].chars().next()?;
    if first.is_whitespace() || first == marker {
        return None;
    }
    // Closer: same marker, preceded by a non-space non-marker char, followed by
    // end / whitespace / punctuation.
    let mut j = at + 1;
    while let Some(rel) = line[j..].find(marker) {
        let pos = j + rel;
        let before = line[..pos].chars().next_back();
        let after = line[pos + marker.len_utf8()..].chars().next();
        let before_ok = before.is_some_and(|c| !c.is_whitespace() && c != marker);
        let after_ok = after.is_none_or(|c| c.is_whitespace() || c.is_ascii_punctuation());
        if pos > at + 1 && before_ok && after_ok {
            let mut base = Run::default();
            match marker {
                '*' => base.bold = true,
                '_' => base.italic = true,
                _ => base.strike = true,
            }
            return Some((at + 1, pos, pos + marker.len_utf8(), base));
        }
        j = pos + marker.len_utf8();
    }
    None
}

fn boundary_before(line: &str, i: usize) -> bool {
    line[..i]
        .chars()
        .next_back()
        .is_none_or(|c| c.is_whitespace() || c.is_ascii_punctuation())
}

/// Splits `text` into runs carrying `base`'s styling, marking resolved
/// `@mentions` (and `@here`/`@channel`/`@everyone`).
fn push_mentions(out: &mut Vec<Run>, text: &str, base: &Run, mentions: &[String]) {
    if text.is_empty() {
        return;
    }
    let mut rest = text;
    while let Some(at) = rest.find('@') {
        let after = &rest[at + 1..];
        let token: String = after
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.')
            .collect();
        if let Some(name) = resolve_mention(&token, mentions) {
            if at > 0 {
                out.push(Run {
                    text: rest[..at].to_string(),
                    ..base.clone()
                });
            }
            out.push(Run {
                text: format!("@{name}"),
                mention: true,
                ..base.clone()
            });
            // `name` is a prefix of the ASCII `token`, so its byte length is a
            // valid boundary in `after`; any trimmed trailing `.` stays plain.
            rest = &after[name.len()..];
        } else {
            // Emit up to and including the `@`, keep scanning.
            let end = at + 1;
            out.push(Run {
                text: rest[..end].to_string(),
                ..base.clone()
            });
            rest = &rest[end..];
        }
    }
    if !rest.is_empty() {
        out.push(Run {
            text: rest.to_string(),
            ..base.clone()
        });
    }
}

/// Whether `name` is a resolved mention or one of the channel specials.
pub fn is_mention(name: &str, mentions: &[String]) -> bool {
    matches!(name, "here" | "channel" | "everyone")
        || mentions.iter().any(|m| m.eq_ignore_ascii_case(name))
}

/// Resolves a `@`-token to the slice that should be highlighted: the token
/// as-is first (so dotted team names like `phoenix.bots` match), then the
/// token with **trailing** `.`s trimmed (so `@alice.` at the end of a sentence
/// still highlights `alice`, leaving the period as plain text). Only trailing
/// dots are stripped, so a real dotted name is never split. `None` when nothing
/// resolves.
fn resolve_mention<'a>(token: &'a str, mentions: &[String]) -> Option<&'a str> {
    let mut cand = token;
    loop {
        if !cand.is_empty() && is_mention(cand, mentions) {
            return Some(cand);
        }
        cand = cand.strip_suffix('.')?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(runs: &[Run]) -> Vec<&str> {
        runs.iter().map(|r| r.text.as_str()).collect()
    }

    #[test]
    fn parses_bold_italic_strike_code() {
        let r = parse_inline("a *b* _c_ ~d~ `e`", &[]);
        assert_eq!(texts(&r), ["a ", "b", " ", "c", " ", "d", " ", "e"]);
        assert!(r[1].bold && !r[1].italic);
        assert!(r[3].italic);
        assert!(r[5].strike);
        assert!(r[7].code);
    }

    #[test]
    fn ignores_non_flanking_and_keeps_markers_literal() {
        // Multiplication, not bold — markers hug spaces / digits on both sides.
        let r = parse_inline("2 * 3 * 4", &[]);
        assert_eq!(r.len(), 1);
        assert!(!r[0].bold);
        assert_eq!(r[0].text, "2 * 3 * 4");
    }

    #[test]
    fn code_is_verbatim_no_inner_markup() {
        let r = parse_inline("see `*not bold*`", &[]);
        assert_eq!(texts(&r), ["see ", "*not bold*"]);
        assert!(r[1].code);
    }

    #[test]
    fn highlights_mentions_inside_and_outside_emphasis() {
        let r = parse_inline("hi @alice and *@bob*", &["alice".into(), "bob".into()]);
        let bob = r.iter().find(|r| r.text == "@bob").unwrap();
        assert!(bob.mention && bob.bold);
        assert!(r.iter().any(|r| r.text == "@alice" && r.mention));
    }

    #[test]
    fn mention_before_a_period_still_highlights() {
        // "thanks @alice." — the trailing sentence period must not swallow the
        // username and suppress the highlight.
        let r = parse_inline("thanks @alice.", &["alice".into()]);
        assert!(
            r.iter().any(|r| r.text == "@alice" && r.mention),
            "@alice should highlight even with a trailing period"
        );
        // The period survives as plain text.
        assert!(r.iter().any(|r| r.text.contains('.') && !r.mention));
    }

    #[test]
    fn dotted_team_mention_is_not_split() {
        // A genuine dotted name resolves whole; trailing-dot trimming must not
        // chop it down to a shorter prefix.
        let r = parse_inline("ping @phoenix.bots", &["phoenix.bots".into()]);
        assert!(r.iter().any(|r| r.text == "@phoenix.bots" && r.mention));
        // And if only the parent resolves, the dotted token is left alone
        // (we only strip *trailing* dots, never interior segments).
        let r2 = parse_inline("ping @phoenix.bots", &["phoenix".into()]);
        assert!(!r2.iter().any(|r| r.mention));
    }

    #[test]
    fn channel_special_with_trailing_period_highlights() {
        let r = parse_inline("heads up @here.", &[]);
        assert!(r.iter().any(|r| r.text == "@here" && r.mention));
    }

    #[test]
    fn unresolved_mention_is_plain() {
        let r = parse_inline("hi @stranger", &["alice".into()]);
        assert!(!r.iter().any(|r| r.mention));
    }
}
