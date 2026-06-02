mod wallpapersclan;
mod wallpaperflare;

use std::collections::HashSet;
use std::path::Path;

const CDN_BASE: &str = "https://raw.githubusercontent.com/yapude/wallpapers/main/assets";

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::sync::{Mutex, Semaphore};

// global stats that all tasks can update without locking
struct Stats {
    downloaded: AtomicU32,
    skipped: AtomicU32,
    failed: AtomicU32,
    pushed: AtomicU32,
}

impl Stats {
    fn new() -> Self {
        Self {
            downloaded: AtomicU32::new(0),
            skipped: AtomicU32::new(0),
            failed: AtomicU32::new(0),
            pushed: AtomicU32::new(0),
        }
    }
}

// get disk usage stats for the runner
fn get_disk_usage() -> String {
    let output_dir = Path::new("assets");
    let mut total_bytes: u64 = 0;
    let mut file_count: u64 = 0;
    if let Ok(entries) = std::fs::read_dir(output_dir) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                if meta.is_file() {
                    total_bytes += meta.len();
                    file_count += 1;
                }
            }
        }
    }
    let mb = total_bytes / (1024 * 1024);
    format!("{}MB across {} files", mb, file_count)
}

fn get_readme_lines(md_file: &str) -> usize {
    std::fs::read_to_string(md_file)
        .map(|c| c.lines().count())
        .unwrap_or(0)
}

// print a compact stats dashboard
fn print_stats(stats: &Stats, md_file: &str) {
    let dl = stats.downloaded.load(Ordering::Relaxed);
    let skip = stats.skipped.load(Ordering::Relaxed);
    let fail = stats.failed.load(Ordering::Relaxed);
    let pushes = stats.pushed.load(Ordering::Relaxed);
    let readme_lines = get_readme_lines(md_file);
    let disk = get_disk_usage();
    println!(
        "[stats] downloaded: {} | skipped: {} | failed: {} | pushes: {} | active readme: {} lines | local disk: {}",
        dl, skip, fail, pushes, readme_lines, disk
    );
}

#[tokio::main]
async fn main() {
    println!("=== site-archive scraper ===");

    // Global limits and locks
    let dl_semaphore = Arc::new(Semaphore::new(30)); // max 30 concurrent downloads across all tags
    let md_mutex = Arc::new(Mutex::new(()));
    let unpushed_count = Arc::new(Mutex::new(0u32));
    let stats = Arc::new(Stats::new());
/* tag storage
    "anime",
    "genshin impact",
    "wuthering waves",
    "artwork",
    "space",
    "anime sexy",
    "blue archive",
    "video games",
-----------------------
 

*/
    // scrape wallpaperflare with specific tags
    let flare_tags = vec![
        "night",
        "graphics",
        "city",
        "architecture",
        "landscape",
        "nature",
        "space",
        "fantasy art",
        "honkai star rail",
        "zenless zone zero",
        "arknights",
        "artistic",
        "water",
        "sky",
        "river",
        "art",
        "trees",
        "minecraft",
        "painting",
        "clouds",
        "beauty in nature",
        "tree",
        "plant",
        "scenics - nature",
        "oil on canvas",
        "tranquility",
        "outside",
        "tranquil scene",
        "country",
        "countryside",
        "day",
        "land",
        "forest",
        "cloud - sky",
        "mountains",
        "mountain",
        "artistry",
        "reflections",
        "lake",
        "scenic",
        "non-urban scene",
        "environment",
        "people",
        "loli",
        "anime girls",
        "ecchi",
        "school uniform",
        "Houkai Gakuen",
        "Kiana Kaslana",
        "thigh-highs",
        "skirt",
        "artwork",
        "weapon",
        "anime",
        "Honkai",
        "backgrounds",
        "computer Graphic",
        "technology",
        "futuristic",
        "vector",
        "illustration",
        "men",
        "fantasy",
        "astronomy",
        "abstract",
        "representation",
        "indoors",
        "still life",
        "art and craft",
        "no people",
        "high angle view",
        "creativity",
        "human representation",
        "celebration",
        "table",
        "multi colored",
        "confetti",
        "decoration",
        "toy",
        "close-up",
        "large group of objects",
        "craft",
        "white",
        "haired",
        "female",
        "character",
        "manga",
        "fan art",
        "minimalism",
        "monochrome",
        "dark background",
        "pantsu shot",
        "uniform",
        "selective coloring",
        "ecchi",
        "Tanaka Kotoha",
        "gyorui",
        "katsuwo drawing",
        "map",
        "thighs",
        "science fiction",
        "sunset",
        "walking",
        "woman",
        "street",
        "lantern",
    ];

    let shared_client = match wallpaperflare::build_client() {
        Ok(c) => Arc::new(c),
        Err(e) => {
            println!("failed to build client: {}", e);
            return;
        }
    };

    let mut tasks = Vec::new();
    for tag in flare_tags {
        let sem = dl_semaphore.clone();
        let mtx = md_mutex.clone();
        let u_count = unpushed_count.clone();
        let s = stats.clone();
        let tag = tag.to_string();
        let client = shared_client.clone();
        tasks.push(tokio::spawn(async move {
            scrape_source(client, "assets", &["README.md", "README2.md"], "README2.md", Some(&tag), u32::MAX, sem, mtx, u_count, s).await;
        }));
    }

    // Wait for all tag scraping tasks to finish
    futures::future::join_all(tasks).await;

    if std::env::var("GITHUB_ACTIONS").is_ok() {
        let _ = std::fs::remove_file(".git/index.lock");
        let _ = tokio::process::Command::new("git").args(["add", "--ignore-removal", "--sparse", "README.md", "README2.md", "assets"])
            .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().await;
        let _ = tokio::process::Command::new("git").args(["commit", "-m", "chore: sort readme alphabetically [skip ci]"])
            .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().await;
        // let _ = tokio::process::Command::new("git").args(["push"])
        //     .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().await;
        let _ = tokio::process::Command::new("git").args(["-c", "http.postBuffer=524288000", "push"]).status().await;
    }   

    println!("=== all scraping complete! ===");
}

async fn scrape_source(
    client: Arc<wreq::Client>,
    source_name: &str,
    read_md_files: &[&str],
    write_md_file: &str,
    search_query: Option<&str>,
    max_pages: u32,
    dl_semaphore: Arc<Semaphore>,
    md_mutex: Arc<Mutex<()>>,
    unpushed_count: Arc<Mutex<u32>>,
    stats: Arc<Stats>
) {
    let tag_label = search_query.unwrap_or("all");
    let output_dir = Path::new(source_name);
    if !output_dir.exists() {
        std::fs::create_dir_all(output_dir).unwrap_or(());
    }

    let mut existing_ids = {
        let _lock = md_mutex.lock().await;
        load_existing_ids(source_name, read_md_files)
    };

    {
        let _lock = md_mutex.lock().await;
        let header = "# Wallpaper Archive\n\nAutomated archive of wallpapers to bypass Cloudflare and prevent dead links.\n\n## Gallery\n\n| Preview | Title | Tags |\n| --- | --- | --- |\n";
        if !Path::new(write_md_file).exists() {
            let _ = std::fs::write(write_md_file, header);
        } else {
            // make sure the table header exists in the file
            // DANGER: never use unwrap_or_default() here! if read_to_string fails due to OOM, 
            // it will return "" and completely overwrite the 100k line file with just the header!
            if let Ok(content) = std::fs::read_to_string(write_md_file) {
                if !content.contains("| --- | --- | --- |") {
                    let _ = std::fs::write(write_md_file, format!("{}{}", header, content));
                }
            } else {
                println!("[warn] failed to read {} to check header, skipping injection", write_md_file);
            }
        }
    }

    let mut total_downloaded = 0u32;
    let mut total_failed = 0u32;
    let mut page = 1u32;
    let mut consecutive_errors = 0u32;
    let max_retries = 3u32;

    loop {
        if page > max_pages {
            break;
        }

        let mut attempt = 0;
        let result = loop {
            attempt += 1;
            let scrape_res = wallpaperflare::scrape_wallpaperflare(&client, 12, page, search_query).await;

            match scrape_res {
                Ok(items) => break Ok(items),
                Err(e) => {
                    if attempt >= max_retries {
                        break Err(e);
                    }
                    let wait = attempt * 5;
                    // println!("[retry] {} page {} attempt {}/{} failed: {} — waiting {}s...", source_name, page, attempt, max_retries, e, wait);
                    tokio::time::sleep(std::time::Duration::from_secs(wait as u64)).await;
                }
            }
        };

        match result {
            Ok(items) => {
                consecutive_errors = 0;

                if items.is_empty() {
                    // println!("[{}] exhausted at page {}", tag_label, page);
                    break;
                }
                let mut page_downloaded = 0;
                let mut new_readme_rows = String::new();

                let mut download_tasks = Vec::new();
                for item in items {
                    let slug = item.id.clone();
                    if existing_ids.contains(&slug) {
                        stats.skipped.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    existing_ids.insert(slug.clone());

                    let output_dir = output_dir.to_path_buf();
                    let max_retries = max_retries;
                    let sem = dl_semaphore.clone();
                    let client = client.clone();

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

                        // silent download — stats printed per batch
                        
                        for dl_attempt in 1..=max_retries {
                            let dl_res = wallpaperflare::download_wallpaper(&client, &item.download_url, &filepath).await;

                            match dl_res {
                                Ok(bytes) => return Ok((slug, ext, item, filename, bytes)),
                                Err(e) => {
                                    // don't retry permanent errors — size rejections etc are not transient
                                    if e.contains("too large") || e.contains("write failed") {
                                        // permanent error, skip silently
                                        let _ = std::fs::remove_file(&manifest_path);
                                        return Err(());
                                    }
                                    if dl_attempt < max_retries {
                                        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                                    } else {
                                        let _ = std::fs::remove_file(&manifest_path);
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
                    if let Ok(Ok((_, _, item, filename, _bytes))) = res {
                        total_downloaded += 1;
                        stats.downloaded.fetch_add(1, Ordering::Relaxed);
                        page_downloaded += 1;

                        let cdn_url = format!("{}/{}", CDN_BASE, filename);
                        let tags = item.tags.join(", ");
                        new_readme_rows.push_str(&format!(
                            "| <img src=\"{}\" width=\"200\"> | **{}**<br>[Download]({}) | {} |\n",
                            cdn_url, item.title, cdn_url, tags
                        ));
                    } else {
                        total_failed += 1;
                        stats.failed.fetch_add(1, Ordering::Relaxed);
                    }
                }

                if page_downloaded > 0 {
                    let _lock = md_mutex.lock().await;
                    append_to_readme(write_md_file, &new_readme_rows);
                    
                    let mut count = unpushed_count.lock().await;
                    *count += page_downloaded;
                    
                    if *count >= 50 {
                        if std::env::var("GITHUB_ACTIONS").is_ok() {
                            println!("[push] freezing downloads to commit batch of {} images...", *count);
                            // acquire all 30 permits to absolutely guarantee NO other tags are downloading
                            // or mutating the assets/ directory while git is scanning it
                            let _freeze = dl_semaphore.acquire_many(30).await.unwrap();
                            
                            let _ = std::fs::remove_file(".git/index.lock");
                            let _ = tokio::process::Command::new("git").args(["add", "--ignore-removal", "--sparse", "README.md", "README2.md", "assets"])
                                .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().await;
                            let _ = tokio::process::Command::new("git").args(["commit", "-m", "chore: archive batch of new wallpapers [skip ci]"])
                                .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().await;
                            let push_status = tokio::process::Command::new("git").args(["-c", "http.postBuffer=524288000", "push"])
                                .status().await;
                            
                            if let Ok(s) = push_status {
                                if s.success() {
                                    stats.pushed.fetch_add(1, Ordering::Relaxed);
                                    println!("[push] success! cleaning up local assets to free disk...");
                                    // nuke local image files after push to free disk space
                                    // keep readme and .git intact obviously
                                    if let Ok(entries) = std::fs::read_dir("assets") {
                                        for entry in entries.flatten() {
                                            let _ = std::fs::remove_file(entry.path());
                                        }
                                    }
                                    print_stats(&stats, write_md_file);
                                } else {
                                    println!("[push] failed! keeping local files for retry");
                                }
                            }
                        }
                        *count = 0;
                    }
                }
            }
            Err(e) => {
                consecutive_errors += 1;
                println!("[error] {} page {} failed after retries: {}", tag_label, page, e);

                if consecutive_errors >= 5 {
                    println!("[halt] {} — too many consecutive failures", tag_label);
                    break;
                }
            }
        }

        page += 1;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    println!("[done] {} — downloaded: {}, failed: {}", tag_label, total_downloaded, total_failed);
}

fn load_existing_ids(source_name: &str, md_files: &[&str]) -> HashSet<String> {
    let mut ids = HashSet::new();
    for md_file in md_files {
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
    }
    ids
}

fn append_to_readme(md_file: &str, rows: &str) {
    // read existing content, trim trailing whitespace to avoid blank lines
    // breaking the markdown table, then append rows directly after
    if let Ok(existing) = std::fs::read_to_string(md_file) {
        let trimmed = existing.trim_end();
        let new_content = format!("{}\n{}", trimmed, rows);
        let _ = std::fs::write(md_file, new_content);
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
