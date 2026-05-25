mod wallpapersclan;
mod wallpaperflare;

use std::collections::HashSet;
use std::path::Path;

const CDN_BASE: &str = "https://raw.githubusercontent.com/yapude/wallpapers/main/assets";

use std::sync::Arc;
use tokio::sync::{Mutex, Semaphore};

#[tokio::main]
async fn main() {
    println!("=== site-archive scraper ===");

    // Global limits and locks
    let dl_semaphore = Arc::new(Semaphore::new(30)); // max 30 concurrent downloads across all tags
    let md_mutex = Arc::new(Mutex::new(()));

    // scrape wallpaperflare with specific tags
    let flare_tags = vec![
        "anime",
        "genshin impact",
        "wuthering waves",
        "artwork",
        "space",
        "anime sexy",
    ];

    let mut tasks = Vec::new();
    for tag in flare_tags {
        let sem = dl_semaphore.clone();
        let mtx = md_mutex.clone();
        let tag = tag.to_string();
        tasks.push(tokio::spawn(async move {
            scrape_source("assets", "README.md", Some(&tag), u32::MAX, sem, mtx).await;
        }));
    }

    // Wait for all tag scraping tasks to finish
    futures::future::join_all(tasks).await;

    if std::env::var("GITHUB_ACTIONS").is_ok() {
        let _ = std::process::Command::new("git").args(["add", "--sparse", "README.md", "assets"]).status();
        let _ = std::process::Command::new("git").args(["commit", "-m", "chore: sort readme alphabetically [skip ci]"]).status();
        let _ = std::process::Command::new("git").args(["push"]).status();
    }

    println!("=== all scraping complete! ===");
}

async fn scrape_source(
    source_name: &str,
    md_file: &str,
    search_query: Option<&str>,
    max_pages: u32,
    dl_semaphore: Arc<Semaphore>,
    md_mutex: Arc<Mutex<()>>
) {
    if let Some(q) = search_query {
        println!("\n--- starting {} (query: {}) ---", source_name, q);
    } else {
        println!("\n--- starting {} ---", source_name);
    }
    let output_dir = Path::new(source_name);
    if !output_dir.exists() {
        std::fs::create_dir_all(output_dir).unwrap_or(());
    }

    let mut existing_ids = {
        let _lock = md_mutex.lock().await;
        load_existing_ids(source_name, md_file)
    };
    println!("found {} already-archived wallpapers for {} in {}", existing_ids.len(), source_name, md_file);

    {
        let _lock = md_mutex.lock().await;
        if !Path::new(md_file).exists() {
            let header = "# Wallpaper Archive\n\nAutomated archive of wallpapers to bypass Cloudflare and prevent dead links.\n\n## Gallery\n\n| Preview | Title | Tags |\n| --- | --- | --- |\n".to_string();
            let _ = std::fs::write(md_file, header);
        }
    }

    let mut total_downloaded = 0u32;
    let mut total_failed = 0u32;
    let mut page = 1u32;
    let mut consecutive_errors = 0u32;
    let max_retries = 3u32;

    loop {
        if page > max_pages {
            println!("reached max_pages ({}) for this run, stopping.", max_pages);
            break;
        }

        if let Some(q) = search_query {
            println!("\n--- {} (query: {}) page {} ---", source_name, q, page);
        } else {
            println!("\n--- {} page {} ---", source_name, page);
        }

        let mut attempt = 0;
        let result = loop {
            attempt += 1;
            let scrape_res = wallpaperflare::scrape_wallpaperflare(12, page, search_query).await;

            match scrape_res {
                Ok(items) => break Ok(items),
                Err(e) => {
                    if attempt >= max_retries {
                        break Err(e);
                    }
                    let wait = attempt * 5;
                    println!("[retry] {} page {} attempt {}/{} failed: {} — waiting {}s...", source_name, page, attempt, max_retries, e, wait);
                    tokio::time::sleep(std::time::Duration::from_secs(wait as u64)).await;
                }
            }
        };

        match result {
            Ok(items) => {
                consecutive_errors = 0;

                if items.is_empty() {
                    println!("no more items found for {}! reached the end at page {}.", source_name, page);
                    break;
                }

                println!("found {} items on {} page {}", items.len(), source_name, page);
                let mut page_downloaded = 0;
                let mut new_readme_rows = String::new();

                let mut download_tasks = Vec::new();
                for item in items {
                    let slug = item.id.clone();
                    if existing_ids.contains(&slug) {
                        println!("  [skip] {} (already archived)", slug);
                        continue;
                    }
                    existing_ids.insert(slug.clone());

                    let output_dir = output_dir.to_path_buf();
                    let max_retries = max_retries;
                    let sem = dl_semaphore.clone();

                    download_tasks.push(tokio::spawn(async move {
                        let _permit = sem.acquire().await.unwrap();
                        let ext = if item.download_url.contains(".png") { "png" } else { "jpg" };
                        let filename = format!("{}.{}", slug, ext);
                        let filepath = output_dir.join(&filename);
                        
                        if filepath.exists() {
                            return Ok((slug, ext, item, filename, 0));
                        }

                        let manifest_path = output_dir.join(format!("{}.json", slug));
                        if let Ok(json) = serde_json::to_string_pretty(&item) {
                            let _ = std::fs::write(&manifest_path, json);
                        }

                        print!("  [dl] {} ... ", filename);
                        
                        for dl_attempt in 1..=max_retries {
                            let dl_res = wallpaperflare::download_wallpaper(&item.download_url, &filepath).await;

                            match dl_res {
                                Ok(bytes) => return Ok((slug, ext, item, filename, bytes)),
                                Err(e) => {
                                    if dl_attempt < max_retries {
                                        print!("retry {}... ", dl_attempt + 1);
                                        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                                    } else {
                                        println!("FAILED after {} attempts: {}", max_retries, e);
                                        return Err(());
                                    }
                                }
                            }
                        }
                        Err(())
                    }));
                }

                let results = futures::future::join_all(download_tasks).await;
                
                for res in results {
                    if let Ok(Ok((_, _, item, filename, bytes))) = res {
                        if bytes > 0 {
                            println!("ok ({} KB)", bytes / 1024);
                        }
                        total_downloaded += 1;
                        page_downloaded += 1;

                        let cdn_url = format!("{}/{}", CDN_BASE, filename);
                        let tags = item.tags.join(", ");
                        new_readme_rows.push_str(&format!(
                            "| <img src=\"{}\" width=\"200\"> | **{}**<br>[Download]({}) | {} |\n",
                            cdn_url, item.title, cdn_url, tags
                        ));
                    } else {
                        total_failed += 1;
                    }
                }

                if page_downloaded > 0 {
                    let _lock = md_mutex.lock().await;
                    append_to_readme(md_file, &new_readme_rows);
                    if std::env::var("GITHUB_ACTIONS").is_ok() {
                        println!("[ci] committing progress for {} page {}...", source_name, page);
                        let _ = std::process::Command::new("git").args(["add", "--sparse", md_file, source_name]).status();
                        let _ = std::process::Command::new("git")
                            .args(["commit", "-m", &format!("chore: archive {} page {} ({} new) [skip ci]", source_name, page, page_downloaded)])
                            .status();
                        let _ = std::process::Command::new("git").args(["push"]).status();
                    }
                }
            }
            Err(e) => {
                consecutive_errors += 1;
                println!("error scraping {} page {} after {} retries: {}", source_name, page, max_retries, e);

                if consecutive_errors >= 5 {
                    println!("too many consecutive failures ({}), halting.", consecutive_errors);
                    break;
                }

                println!("skipping page {} and continuing...", page);
            }
        }

        page += 1;
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    println!("\n=== {} done! downloaded: {}, failed: {} ===", source_name, total_downloaded, total_failed);
}

fn load_existing_ids(source_name: &str, md_file: &str) -> HashSet<String> {
    let mut ids = HashSet::new();
    if let Ok(content) = std::fs::read_to_string(md_file) {
        for line in content.lines() {
            let search_str = format!("/{}/", source_name);
            if let Some(start) = line.find(&search_str) {
                let after = &line[start + search_str.len()..];
                if let Some(dot) = after.find('.') {
                    let slug = &after[..dot];
                    if !slug.is_empty() {
                        ids.insert(slug.to_string());
                    }
                }
            }
        }
    }
    ids
}

fn append_to_readme(md_file: &str, rows: &str) {
    use std::fs::OpenOptions;
    use std::io::Write;

    if let Ok(mut file) = OpenOptions::new().append(true).open(md_file) {
        let _ = file.write_all(rows.as_bytes());
        println!("appended {} new entries to {}", rows.lines().count(), md_file);
    }
}

#[allow(dead_code)]
fn sort_readme(md_file: &str) {
    let content = match std::fs::read_to_string(md_file) {
        Ok(c) => c,
        Err(_) => return,
    };

    let lines: Vec<&str> = content.lines().collect();

    let mut header_lines = Vec::new();
    let mut data_rows = Vec::new();

    for line in &lines {
        if line.starts_with("| <img") {
            data_rows.push(*line);
        } else {
            if data_rows.is_empty() {
                header_lines.push(*line);
            }
        }
    }

    data_rows.sort();

    let mut output = header_lines.join("\n");
    output.push('\n');
    for row in &data_rows {
        output.push_str(row);
        output.push('\n');
    }

    let _ = std::fs::write(md_file, output);
    println!("sorted readme: {} entries alphabetically in {}", data_rows.len(), md_file);
}
