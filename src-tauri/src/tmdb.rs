use scraper::{ElementRef, Html, Selector};
use serde::Serialize;
use std::collections::HashSet;
use std::time::Duration;

const MAX_HTML_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TmdbMatch {
    id: u64,
    title: String,
    original_title: String,
    year: Option<String>,
    overview: String,
    kind: String,
}

fn validate(query: &str, kind: &str) -> Result<(), String> {
    if !matches!(kind, "movie" | "tv") {
        return Err("请选择电影或电视剧。".into());
    }
    if query.is_empty() || query.chars().count() > 200 || query.chars().any(char::is_control) {
        return Err("搜索词应为 1–200 个有效字符。".into());
    }
    Ok(())
}

fn text_of(element: ElementRef<'_>) -> String {
    element
        .text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn year_of(date: &str) -> Option<String> {
    date.split(|c: char| !c.is_ascii_digit())
        .find(|part| {
            part.len() == 4
                && part
                    .parse::<u16>()
                    .is_ok_and(|n| (1800..=2199).contains(&n))
        })
        .map(str::to_owned)
}

fn parse_id(href: &str, kind: &str) -> Option<u64> {
    let tail = href.strip_prefix(&format!("/{kind}/"))?;
    let digits = tail.split(['-', '/', '?', '#']).next()?;
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
        .then(|| digits.parse().ok())
        .flatten()
}

fn parse_card(
    card: ElementRef<'_>,
    kind: &str,
    anchor_sel: &Selector,
    h2_sel: &Selector,
    span_sel: &Selector,
    date_sel: &Selector,
    p_sel: &Selector,
) -> Option<TmdbMatch> {
    let (anchor, heading) = card.select(anchor_sel).find_map(|a| {
        let href = a.value().attr("href")?;
        parse_id(href, kind)?;
        let h2 = a.select(h2_sel).next()?;
        Some((a, h2))
    })?;
    let id = parse_id(anchor.value().attr("href")?, kind)?;
    let spans = heading.select(span_sel).collect::<Vec<_>>();
    let title = if let Some(first) = spans.first() {
        text_of(*first)
    } else {
        text_of(heading)
    };
    if title.is_empty() {
        return None;
    }
    let original_title = spans
        .get(1)
        .map(|span| text_of(*span))
        .map(|value| {
            value
                .trim()
                .trim_start_matches('(')
                .trim_end_matches(')')
                .trim()
                .to_owned()
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| title.clone());
    let year = card
        .select(date_sel)
        .next()
        .and_then(|date| year_of(&text_of(date)));
    let overview = card.select(p_sel).next().map(text_of).unwrap_or_default();
    Some(TmdbMatch {
        id,
        title,
        original_title,
        year,
        overview,
        kind: kind.to_owned(),
    })
}

fn looks_blocked(document: &Html) -> bool {
    let title_sel = Selector::parse("title").unwrap();
    let title = document
        .select(&title_sel)
        .next()
        .map(text_of)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if [
        "just a moment",
        "access denied",
        "captcha",
        "verify you are human",
        "security check",
    ]
    .iter()
    .any(|word| title.contains(word))
    {
        return true;
    }
    let challenge_sel = Selector::parse(
        "#challenge-running, #cf-challenge-running, form[action*='captcha'], #captcha",
    )
    .unwrap();
    if document.select(&challenge_sel).next().is_some() {
        return true;
    }
    let body_sel = Selector::parse("body").unwrap();
    let body = document
        .select(&body_sel)
        .next()
        .map(text_of)
        .unwrap_or_default()
        .to_ascii_lowercase();
    [
        "verify you are human",
        "access to this page has been denied",
        "complete the security check",
        "请完成安全验证",
    ]
    .iter()
    .any(|phrase| body.contains(phrase))
}

fn empty_results(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "no results",
        "no movies",
        "no tv shows",
        "did not match",
        "there are no",
        "没有结果",
        "没有找到",
        "无搜索结果",
        "找不到",
    ]
    .iter()
    .any(|phrase| lower.contains(phrase))
}

fn parse_search_html(html: &str, kind: &str) -> Result<Vec<TmdbMatch>, String> {
    let document = Html::parse_document(html);
    if looks_blocked(&document) {
        return Err("TMDb 要求安全验证，当前无法读取公开搜索结果。请稍后重试。".into());
    }
    let section_sel = Selector::parse(if kind == "movie" {
        "#movie_results"
    } else {
        "#tv_results"
    })
    .unwrap();
    let section = document
        .select(&section_sel)
        .next()
        .ok_or("TMDb 搜索页面结构已变化，暂时无法识别结果。")?;
    let card_sel = Selector::parse("div[class~='comp:media-card'], div.card.v4").unwrap();
    let anchor_sel = Selector::parse("a[href]").unwrap();
    let h2_sel = Selector::parse("h2").unwrap();
    let span_sel = Selector::parse("span").unwrap();
    let date_sel = Selector::parse(".release_date").unwrap();
    let p_sel = Selector::parse("p").unwrap();
    let cards = section.select(&card_sel).collect::<Vec<_>>();
    if cards.is_empty() {
        if empty_results(&text_of(section)) {
            return Ok(Vec::new());
        }
        return Err("TMDb 搜索页面结构已变化，暂时无法识别结果。".into());
    }
    let mut seen = HashSet::new();
    let results = cards
        .into_iter()
        .filter_map(|card| {
            parse_card(
                card,
                kind,
                &anchor_sel,
                &h2_sel,
                &span_sel,
                &date_sel,
                &p_sel,
            )
        })
        .filter(|item| seen.insert(item.id))
        .take(20)
        .collect::<Vec<_>>();
    if results.is_empty() {
        return Err("TMDb 搜索结果格式已变化，暂时无法识别条目。".into());
    }
    Ok(results)
}

// Reads only TMDb's public search page. No account, API key, cookies, or local files are sent.
#[tauri::command]
pub async fn search_tmdb(query: String, kind: String) -> Result<Vec<TmdbMatch>, String> {
    let query = query.trim();
    validate(query, &kind)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .connect_timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("Reelink/0.1 (public TMDb search)")
        .build()
        .map_err(|_| "无法初始化 TMDb 连接。".to_string())?;
    let mut response = client
        .get(format!("https://www.themoviedb.org/search/{kind}"))
        .query(&[("query", query), ("language", "zh-CN")])
        .send()
        .await
        .map_err(|err| {
            if err.is_timeout() {
                "TMDb 连接超时，请稍后重试。"
            } else {
                "无法连接 TMDb，请检查网络。"
            }
            .to_string()
        })?;
    match response.status().as_u16() {
        200 => {}
        401 | 403 | 429 | 503 => {
            return Err("TMDb 暂时限制了公开搜索或要求验证，请稍后重试。".into())
        }
        300..=399 => return Err("TMDb 将搜索请求重定向到了其他页面，已停止读取。".into()),
        _ => return Err("TMDb 搜索暂时不可用，请稍后重试。".into()),
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_HTML_BYTES as u64)
    {
        return Err("TMDb 搜索页面过大，已停止读取。".into());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "读取 TMDb 搜索页面失败。")?
    {
        if body.len() + chunk.len() > MAX_HTML_BYTES {
            return Err("TMDb 搜索页面过大，已停止读取。".into());
        }
        body.extend_from_slice(&chunk);
    }
    let html = String::from_utf8(body).map_err(|_| "TMDb 搜索页面编码无法识别。")?;
    parse_search_html(&html, &kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_query_and_kind() {
        assert!(validate("星际穿越", "movie").is_ok());
        assert!(validate("x", "../tv").is_err());
        assert!(validate("", "movie").is_err());
        assert!(validate(&"x".repeat(201), "tv").is_err());
        assert!(validate("a\nb", "tv").is_err());
    }

    #[test]
    fn parses_only_requested_section_entities_year_and_deduplication() {
        let html = r#"<html><div id="movie_results"><div class="comp:media-card" data-object-id="opaque"><a href="/movie/603-the-matrix?language=zh-CN"><h2><span>黑客&amp;帝国</span><span> (The Matrix)</span></h2></a><span class="release_date">1999 年 05 月</span><p>现实 &amp; 梦境</p></div><div class="comp:media-card"><a href="/movie/603-again"><h2><span>重复</span></h2></a></div></div><div id="tv_results"><div class="comp:media-card"><a href="/tv/1396-breaking-bad"><h2><span>绝命毒师</span><span> (Breaking Bad)</span></h2></a><span class="release_date">2008-01-20</span></div></div></html>"#;
        let movies = parse_search_html(html, "movie").unwrap();
        assert_eq!(movies.len(), 1);
        assert_eq!(
            (
                movies[0].id,
                movies[0].title.as_str(),
                movies[0].original_title.as_str(),
                movies[0].year.as_deref(),
                movies[0].overview.as_str()
            ),
            (603, "黑客&帝国", "The Matrix", Some("1999"), "现实 & 梦境")
        );
        let tv = parse_search_html(html, "tv").unwrap();
        assert_eq!(tv.len(), 1);
        assert_eq!((tv[0].id, tv[0].kind.as_str()), (1396, "tv"));
    }

    #[test]
    fn supports_older_card_markup_and_missing_optional_fields() {
        let html = r#"<div id="tv_results"><div class="card v4 tight"><div class="details"><a href="/tv/1396-breaking-bad"><h2>Breaking Bad</h2></a><span class="release_date">2008-01-20</span><p></p></div></div></div>"#;
        let items = parse_search_html(html, "tv").unwrap();
        assert_eq!(items[0].original_title, "Breaking Bad");
        assert_eq!(items[0].year.as_deref(), Some("2008"));
        assert_eq!(items[0].overview, "");
    }

    #[test]
    fn distinguishes_empty_results_from_changed_markup_and_blocks() {
        assert!(parse_search_html(
            "<div id='movie_results'><p>There are no movies that matched your query.</p></div>",
            "movie"
        )
        .unwrap()
        .is_empty());
        assert!(parse_search_html(
            "<div id='movie_results'><div class='unknown-card'></div></div>",
            "movie"
        )
        .is_err());
        assert!(parse_search_html("<html><div id='other_results'></div></html>", "movie").is_err());
        assert!(parse_search_html(
            "<title>Just a moment...</title><div id='movie_results'></div>",
            "movie"
        )
        .unwrap_err()
        .contains("验证"));
        assert!(parse_search_html("<div id='movie_results'><div class='comp:media-card'><a href='/movie/not-an-id'><h2>Bad</h2></a></div></div>", "movie").is_err());
    }

    #[test]
    #[ignore = "requires the public TMDb website"]
    fn live_public_movie_search() {
        let items =
            tauri::async_runtime::block_on(search_tmdb("The Matrix".into(), "movie".into()))
                .unwrap();
        assert!(items.iter().any(|item| item.id == 603));
    }

    #[test]
    #[ignore = "requires the public TMDb website"]
    fn live_public_tv_search() {
        let items = tauri::async_runtime::block_on(search_tmdb("Breaking Bad".into(), "tv".into()))
            .unwrap();
        assert!(items.iter().any(|item| item.id == 1396));
    }
}
