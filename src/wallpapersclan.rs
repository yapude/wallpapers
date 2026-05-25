use scraper::{Html, Selector};
use std::path::Path;
use wreq_util::Emulation;

const BASE_URL: &str = "https://wallpapers-clan.com";
const DESKTOP_URL: &str = "https://wallpapers-clan.com/desktop-wallpapers/";

use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize)]
pub struct WallpaperEntry {
    pub id: String,
    pub title: String,
    pub thumbnail_url: String,
    pub detail_url: String,
    pub download_url: String,
    pub tags: Vec<String>,
}

/// build a wreq client impersonating chrome — no cookies needed,
/// the tls fingerprint alone bypasses cloudflare's managed challenge
fn build_client() -> Result<wreq::Client, String> {
    wreq::Client::builder()
        .emulation(Emulation::Chrome134)
        .cookie_store(true)
        .build()
        .map_err(|e| e.to_string())
}

/// scrape the listing page and resolve download urls from detail pages
pub async fn scrape_wallpapersclan(
    limit: usize,
    page: u32,
) -> Result<Vec<WallpaperEntry>, String> {
    let client = build_client()?;

    let (url, response) = if page > 1 {
        let ajax_url = format!("{}/wp-admin/admin-ajax.php", BASE_URL);
        println!("[listing] fetching (ajax): {} (page {})", ajax_url, page);

        let mut form = std::collections::HashMap::new();
        form.insert("action", "boldlab_get_new_posts");
        form.insert("options[plugin]", "boldlab_core");
        form.insert("options[module]", "post-types/dwallpapers/shortcodes");
        form.insert("options[shortcode]", "dwallpapers-list");
        form.insert("options[post_type]", "dwallpapers");
        let page_str = page.to_string();
        form.insert("options[next_page]", &page_str);
        form.insert("options[max_pages_num]", "863");
        form.insert("options[show_category]", "no");
        form.insert("options[behavior]", "columns");
        form.insert("options[images_proportion]", "full");
        form.insert("options[columns]", "3");
        form.insert("options[space]", "normal");
        form.insert("options[columns_responsive]", "predefined");
        form.insert("options[columns_1440]", "3");
        form.insert("options[columns_1366]", "3");
        form.insert("options[columns_1024]", "3");
        form.insert("options[columns_768]", "3");
        form.insert("options[columns_680]", "3");
        form.insert("options[columns_480]", "3");
        form.insert("options[posts_per_page]", "12");
        form.insert("options[orderby]", "date");
        form.insert("options[order]", "DESC");
        form.insert("options[additional_params]", "tax");
        form.insert("options[layout]", "info-below");
        form.insert("options[hover_animation_info-below]", "tilt");
        form.insert("options[hover_animation_info-follow]", "follow");
        form.insert("options[hover_animation_info-on-hover]", "direction-aware");
        form.insert("options[title_tag]", "h4");
        form.insert("options[custom_padding]", "no");
        form.insert("options[enable_filter]", "yes");
        form.insert("options[pagination_type]", "infinite-scroll");
        form.insert("options[loading_animation]", "no");
        form.insert("options[object_class_name]", "BoldlabCoredwallpapersListShortcode");
        form.insert("options[taxonomy_filter]", "dwallpapers-category");
        form.insert("options[space_value]", "15");
        form.insert("options[justified_attr]", "{\"rowHeight\":\"\",\"spaceBetween\":15}");

        let resp = client
            .post(&ajax_url)
            .header("Referer", DESKTOP_URL)
            .header("X-Requested-With", "XMLHttpRequest")
            .form(&form)
            .timeout(std::time::Duration::from_secs(20))
            .send()
            .await
            .map_err(|e| format!("ajax request failed: {}", e))?;
        
        (ajax_url, resp)
    } else {
        println!("[listing] fetching: {}", DESKTOP_URL);
        let resp = client
            .get(DESKTOP_URL)
            .header("Referer", BASE_URL)
            .timeout(std::time::Duration::from_secs(20))
            .send()
            .await
            .map_err(|e| format!("request failed: {}", e))?;
        (DESKTOP_URL.to_string(), resp)
    };

    let status = response.status();
    println!("[listing] http {}", status);

    if !status.is_success() {
        return Err(format!("HTTP {} from wallpapers-clan", status));
    }

    let raw_text = response.text().await.map_err(|e| e.to_string())?;
    
    let html = if page > 1 {
        println!("[listing] raw response: {:.200}", raw_text);
        // the ajax response is JSON with a "data" string containing the HTML
        let json: serde_json::Value = serde_json::from_str(&raw_text)
            .map_err(|e| format!("failed to parse ajax json: {}", e))?;
        json["data"]
            .as_str()
            .unwrap_or("")
            .to_string()
    } else {
        raw_text
    };

    // first pass: collect listing data
    let mut listing_items: Vec<(String, String, String, Vec<String>)> = Vec::new();

    {
        let document = Html::parse_document(&html);

        // selectors for the qodef grid layout
        let article_selector = Selector::parse("article.qodef-grid-item").unwrap();
        let media_link_selector = Selector::parse(".qodef-e-media-image a[itemprop='url']").unwrap();
        let img_selector = Selector::parse("img.wp-post-image").unwrap();
        let noscript_selector = Selector::parse("noscript").unwrap();
        let title_selector = Selector::parse("h4.qodef-e-title a.qodef-e-title-link").unwrap();
        let category_selector = Selector::parse(".qodef-e-info-category a.qodef-e-category").unwrap();

        let articles: Vec<_> = document.select(&article_selector).collect();
        println!("[listing] found {} articles", articles.len());

    for article in articles.iter() {
        if listing_items.len() >= limit {
            break;
        }

        // detail page url
        let detail_url = match article.select(&media_link_selector).next() {
            Some(a) => match a.value().attr("href") {
                Some(href) if href.contains("desktop-wallpapers") => href.to_string(),
                _ => continue,
            },
            None => continue,
        };

        // thumbnail — data-lazy-src > data-lazy-srcset > noscript fallback
        let thumbnail_url = article
            .select(&img_selector)
            .next()
            .and_then(|img| {
                if let Some(src) = img.value().attr("data-lazy-src") {
                    if !src.contains("data:image/svg") {
                        return Some(src.to_string());
                    }
                }
                if let Some(srcset) = img.value().attr("data-lazy-srcset") {
                    if let Some(first) = srcset.split(',').next() {
                        let url = first.trim().split_whitespace().next().unwrap_or("");
                        if !url.is_empty() && !url.contains("data:image/svg") {
                            return Some(url.to_string());
                        }
                    }
                }
                if let Some(s) = img.value().attr("src") {
                    if !s.contains("data:image/svg") {
                        return Some(s.to_string());
                    }
                }
                None
            })
            .or_else(|| {
                article.select(&noscript_selector).next().and_then(|ns| {
                    let inner = ns.inner_html();
                    let frag = Html::parse_fragment(&inner);
                    let img_sel = Selector::parse("img").unwrap();
                    frag.select(&img_sel).next().and_then(|img| {
                        img.value()
                            .attr("src")
                            .filter(|s| !s.contains("data:image/svg"))
                            .map(|s| s.to_string())
                    })
                })
            });

        let thumbnail_url = match thumbnail_url {
            Some(url) => url,
            None => continue,
        };

        // title
        let title = article
            .select(&title_selector)
            .next()
            .map(|t| t.text().collect::<String>().trim().to_string())
            .unwrap_or_else(|| "Untitled".to_string());

        // tags from categories
        let tags: Vec<String> = article
            .select(&category_selector)
            .map(|cat| cat.text().collect::<String>().trim().to_string())
            .filter(|t| !t.is_empty())
            .collect();

        listing_items.push((detail_url, thumbnail_url, title, tags));
        }
    }

    println!("[listing] collected {} items, resolving download urls...", listing_items.len());

    // second pass: resolve download urls from detail pages concurrently
    let mut handles = Vec::new();
    for (detail_url, thumb, title, tags) in listing_items {
        let client = client.clone();
        let detail = detail_url.clone();
        handles.push(tokio::spawn(async move {
            let download_url = resolve_download(&client, &detail).await;
            (detail_url, thumb, title, tags, download_url)
        }));
    }

    let mut items = Vec::new();
    for handle in handles {
        match handle.await {
            Ok((detail_url, thumbnail_url, title, tags, download_result)) => {
                // slug for id
                let slug = detail_url
                    .trim_end_matches('/')
                    .split('/')
                    .next_back()
                    .unwrap_or("unknown")
                    .to_string();

                let download_url = match download_result {
                    Ok(url) => url,
                    Err(e) => {
                        println!("  [warn] failed to resolve {}: {}", slug, e);
                        // fallback to thumbnail as download
                        thumbnail_url.clone()
                    }
                };

                items.push(WallpaperEntry {
                    id: slug,
                    title,
                    thumbnail_url,
                    detail_url,
                    download_url,
                    tags,
                });
            }
            Err(e) => {
                println!("  [warn] task failed: {}", e);
            }
        }
    }

    println!("[listing] resolved {} download urls", items.len());
    Ok(items)
}

/// resolve the actual download url from a detail page
/// grabs a.wpdm-download-link[data-downloadurl] — baked in by wordpress
async fn resolve_download(client: &wreq::Client, detail_url: &str) -> Result<String, String> {
    let response = client
        .get(detail_url)
        .header("Referer", DESKTOP_URL)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| format!("request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }

    let html = response.text().await.map_err(|e| e.to_string())?;
    let document = Html::parse_document(&html);

    // primary: wpdm download button with data-downloadurl
    let download_btn = Selector::parse("a.wpdm-download-link").unwrap();
    if let Some(btn) = document.select(&download_btn).next() {
        if let Some(url) = btn.value().attr("data-downloadurl") {
            if !url.is_empty() {
                return Ok(url.to_string());
            }
        }
    }

    // fallback: any download link
    let fallback = Selector::parse(".media-body a[href*='download']").unwrap();
    if let Some(link) = document.select(&fallback).next() {
        if let Some(href) = link.value().attr("href") {
            return Ok(href.to_string());
        }
    }

    // last resort: full-res image on the page
    let img_sel = Selector::parse("img.wp-post-image").unwrap();
    if let Some(img) = document.select(&img_sel).next() {
        let src = img
            .value()
            .attr("data-lazy-src")
            .or_else(|| {
                img.value().attr("data-lazy-srcset").and_then(|srcset| {
                    srcset.split(',').next().and_then(|s| s.trim().split_whitespace().next())
                })
            })
            .or_else(|| img.value().attr("src"))
            .filter(|s| !s.contains("data:image/svg"));

        if let Some(url) = src {
            return Ok(url.to_string());
        }
    }

    Err("no download url found".to_string())
}

/// download a file to disk, returns bytes written
pub async fn download_wallpaper(url: &str, path: &Path) -> Result<u64, String> {
    let client = build_client()?;

    let response = client
        .get(url)
        .header("Referer", DESKTOP_URL)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| format!("download request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }

    let bytes = response.bytes().await.map_err(|e| e.to_string())?;
    let len = bytes.len() as u64;

    std::fs::write(path, &bytes).map_err(|e| format!("write failed: {}", e))?;

    Ok(len)
}
