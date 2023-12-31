use audiotags::Tag;
use clap::{command, Parser};
use fs_extra::dir::CopyOptions;
use regex::Regex;
use reqwest::{header, Client};
use serde::Deserialize;
use std::ops::{Bound, RangeBounds};
use std::{
    fs::{self},
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

trait StringUtils {
    fn substring(&self, start: usize, len: usize) -> &str;
    fn slice(&self, range: impl RangeBounds<usize>) -> &str;
}

impl StringUtils for str {
    fn substring(&self, start: usize, len: usize) -> &str {
        let mut char_pos = 0;
        let mut byte_start = 0;
        let mut it = self.chars();
        loop {
            if char_pos == start {
                break;
            }
            if let Some(c) = it.next() {
                char_pos += 1;
                byte_start += c.len_utf8();
            } else {
                break;
            }
        }
        char_pos = 0;
        let mut byte_end = byte_start;
        loop {
            if char_pos == len {
                break;
            }
            if let Some(c) = it.next() {
                char_pos += 1;
                byte_end += c.len_utf8();
            } else {
                break;
            }
        }
        &self[byte_start..byte_end]
    }
    fn slice(&self, range: impl RangeBounds<usize>) -> &str {
        let start = match range.start_bound() {
            Bound::Included(bound) | Bound::Excluded(bound) => *bound,
            Bound::Unbounded => 0,
        };
        let len = match range.end_bound() {
            Bound::Included(bound) => *bound + 1,
            Bound::Excluded(bound) => *bound,
            Bound::Unbounded => self.len(),
        } - start;
        self.substring(start, len)
    }
}

fn save_bytes_to_file(bytes: &[u8], path: &PathBuf) {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open(path)
        .unwrap();

    let _ = file.write_all(bytes);
}

async fn copy_album_dir_contents(
    target_dir: Option<String>,
    path: PathBuf,
    client: &Client,
    fetch_covers: bool,
    tidal_auth: Option<String>,
) -> Option<String> {
    let files = fs::read_dir(path.clone())
        .unwrap()
        .filter_map(|p| p.ok())
        .collect::<Vec<_>>();

    let music_file_pattern = Regex::new(r".+\.(flac|m4a|mp3)").unwrap();
    let audio_files = files
        .iter()
        .filter(|p| music_file_pattern.is_match(p.file_name().to_str().unwrap()))
        .collect::<Vec<_>>();

    if audio_files.is_empty() {
        println!(
            "Encountered empty directory {}",
            path.clone().to_str().unwrap()
        );
        return None;
    }

    let music_file = audio_files.first().unwrap();

    let tag = Tag::new()
        .read_from_path(music_file.path().to_str().unwrap())
        .unwrap();

    let title = tag.title().unwrap();
    let album = tag.album_title().unwrap_or("(none)");
    let artist = tag.artist().or(tag.album_artist()).unwrap();
    let album_dir_name = path.file_name().unwrap().to_str().unwrap();

    println!("====== {} ======", path.clone().to_str().unwrap());
    println!("file: {}", music_file.path().to_str().unwrap());
    println!("title: {}", title);
    println!("album title: {}", album);
    println!("album directory name: {}", album_dir_name);
    println!("artist: {}", artist);
    println!("description: {:?}", tag.description());
    println!("comment: {:?}", tag.comment());

    let mut created_new_cover = false;

    let contains_album_cover = files.iter().any(|f| {
        f.file_name()
            .to_str()
            .is_some_and(|n| n.starts_with("cover."))
    });
    let contains_artist_cover = files.iter().any(|f| {
        f.file_name()
            .to_str()
            .is_some_and(|n| n.starts_with("artist."))
    });

    if fetch_covers && (!contains_album_cover || !contains_artist_cover) {
        if let Some(tidal_auth) = &tidal_auth {
            if let Some(description) = tag.description().or(tag.comment()) {
                let tidal_prefix = "https://listen.tidal.com/album/";
                if description.starts_with(tidal_prefix) {
                    let remainder = description.strip_prefix(tidal_prefix).unwrap();
                    let tidal_album_id = remainder.substring(0, remainder.find('/').unwrap());
                    let request_url = format!("https://listen.tidal.com/v1/albums/{tidal_album_id}?countryCode=US&locale=en_US&deviceType=BROWSER");
                    println!("Fetching from {request_url}");

                    if let Some(resp) = match client
                        .get(request_url)
                        .header("Authorization", format!("Bearer {tidal_auth}"))
                        .send()
                        .await
                        .unwrap()
                        .json::<serde_json::Value>()
                        .await
                    {
                        Ok(resp) => Some(resp),
                        Err(err) => {
                            eprintln!("Deserialization failure {:?}", err);
                            None
                        }
                    } {
                        if !contains_artist_cover {
                            if let Some(artist) = resp.get("artist") {
                                if let Some(artist) = artist.as_object() {
                                    if let Some(artist_pic) = artist.get("picture") {
                                        if artist_pic.is_null() {
                                            println!("No Artist picture associated with artist");
                                        }
                                        if let Some(artist_pic_path) = artist_pic.as_str() {
                                            let artist_pic_path = artist_pic_path.replace('-', "/");

                                            let request_url = format!(
                                                "https://resources.tidal.com/images/{artist_pic_path}/750x750.jpg"
                                            );
                                            println!("Fetching from {request_url}");

                                            if let Some(resp) = match client
                                                .get(request_url)
                                                .send()
                                                .await
                                            {
                                                Ok(resp) => Some(resp),
                                                Err(err) => {
                                                    eprintln!(
                                                        "Failed to fetch tidal artist picture: {:?}",
                                                        err
                                                    );
                                                    None
                                                }
                                            } {
                                                match resp.bytes().await {
                                                    Ok(bytes) => {
                                                        let cover_file_path =
                                                            path.join("artist.jpg");
                                                        save_bytes_to_file(
                                                            &bytes,
                                                            &cover_file_path,
                                                        );
                                                        created_new_cover = true;
                                                    }
                                                    Err(error) => {
                                                        eprintln!(
                                                            "Deserialization failure {:?}",
                                                            error
                                                        )
                                                    }
                                                };
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if !contains_album_cover {
                            if let Some(cover) = resp.get("cover") {
                                if let Some(cover) = cover.as_str() {
                                    let cover_path = cover.replace('-', "/");
                                    let request_url = format!(
                                        "https://resources.tidal.com/images/{cover_path}/1280x1280.jpg"
                                    );
                                    println!("Fetching from {request_url}");

                                    if let Some(resp) = match client.get(request_url).send().await {
                                        Ok(resp) => Some(resp),
                                        Err(err) => {
                                            eprintln!(
                                                "Failed to fetch tidal artist album: {:?}",
                                                err
                                            );
                                            None
                                        }
                                    } {
                                        match resp.bytes().await {
                                            Ok(bytes) => {
                                                let cover_file_path = path.join("cover.jpg");
                                                save_bytes_to_file(&bytes, &cover_file_path);
                                                created_new_cover = true;
                                            }
                                            Err(error) => {
                                                eprintln!("Deserialization failure {:?}", error)
                                            }
                                        };
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        if tidal_auth.is_some() && !created_new_cover {
            eprintln!("Failed to fetch Tidal artist album");
        } else if !created_new_cover {
            let re = Regex::new(r"[^A-Za-z0-9 _]").unwrap();
            let request_url = format!(
                "http://musicbrainz.org/ws/2/release/?query=artist:{}%20AND%20title:{}%20AND%20packaging:None",
                re.replace_all(artist, "").replace(' ', "%20"),
                re.replace_all(album, "").replace(' ', "%20"),
            );
            println!("Fetching from {request_url}",);
            if let Some(resp) = match client
                .get(request_url)
                .send()
                .await
                .unwrap()
                .json::<serde_json::Value>()
                .await
            {
                Ok(resp) => Some(resp),
                Err(err) => {
                    eprintln!("Failed to fetch artist album: {:?}", err);
                    None
                }
            } {
                if let Some(releases) = resp.get("releases") {
                    if let Some(releases) = releases.as_array() {
                        if let Some(release) = releases.first() {
                            if let Some(id) = release.get("id") {
                                if let Some(id) = id.as_str() {
                                    let request_url =
                                        format!("http://coverartarchive.org/release/{id}");
                                    println!("Fetching {request_url}");
                                    if let Some(resp) = match client
                                        .get(request_url)
                                        .send()
                                        .await
                                        .unwrap()
                                        .json::<serde_json::Value>()
                                        .await
                                    {
                                        Ok(resp) => Some(resp),
                                        Err(err) => {
                                            eprintln!("Failed to fetch artist album: {:?}", err);
                                            None
                                        }
                                    } {
                                        if let Some(images) = resp.get("images") {
                                            if let Some(images) = images.as_array() {
                                                if let Some(image) = images.first() {
                                                    if let Some(main_image) = image.get("image") {
                                                        if let Some(main_image) =
                                                            main_image.as_str()
                                                        {
                                                            let ext_index =
                                                                main_image.rfind('.').unwrap() + 1;
                                                            let end_index = main_image.len();
                                                            let extension = main_image
                                                                .slice(ext_index..end_index);
                                                            let cover_file_path = path.join(
                                                                format!("cover.{}", extension),
                                                            );
                                                            if let Some(resp) = match client
                                                                .get(main_image)
                                                                .send()
                                                                .await
                                                            {
                                                                Ok(resp) => Some(resp),
                                                                Err(err) => {
                                                                    eprintln!("Failed to fetch artist album: {:?}", err);
                                                                    None
                                                                }
                                                            } {
                                                                match resp.bytes().await {
                                                                    Ok(bytes) => {
                                                                        save_bytes_to_file(
                                                                            &bytes,
                                                                            &cover_file_path,
                                                                        );
                                                                        created_new_cover = true;
                                                                    }
                                                                    Err(error) => eprintln!(
                                                                        "Deserialization failure {:?}",
                                                                        error
                                                                    ),
                                                                };
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if let Some(target_dir) = target_dir {
        let artist_dir = Path::new(&target_dir).join(artist);

        if !artist_dir.is_dir() {
            println!("Creating artist dir {}", artist_dir.to_str().unwrap());
            let _ = fs::create_dir(artist_dir.clone());
        }

        let album_dir = artist_dir.join(album_dir_name);

        let existing_files = if album_dir.is_dir() {
            fs::read_dir(album_dir.clone())
                .unwrap()
                .filter_map(|p| p.ok())
                .collect::<Vec<_>>()
        } else {
            vec![]
        };

        if created_new_cover || files.len() > existing_files.len() {
            let source = path.to_str().unwrap();
            let target = artist_dir.to_str().unwrap();
            println!("Copying album dir {} -> {}", source, target);
            if !album_dir.is_dir() {
                let _ = fs_extra::dir::copy(source, target, &CopyOptions::new());
                Some(String::from(album_dir.to_str().unwrap()))
            } else {
                let copied_files = fs::read_dir(path.clone())
                    .unwrap()
                    .filter_map(|p| p.ok())
                    .filter_map(|source| {
                        let target = Path::new(album_dir.to_str().unwrap())
                            .join(source.path().file_name().unwrap().to_str().unwrap());

                        if target.is_file() {
                            None
                        } else {
                            Some((source.path(), target))
                        }
                    })
                    .map(|(source, target)| {
                        let track_source = source.to_str().unwrap();
                        let track_target = target.to_str().unwrap();
                        let _ = fs::copy(track_source, track_target);
                        format!("\t{}", source.file_name().unwrap().to_str().unwrap())
                    })
                    .collect::<Vec<_>>();

                Some(format!(
                    "{}\n{}",
                    album_dir.to_str().unwrap(),
                    copied_files.join("\n"),
                ))
            }
        } else {
            None
        }
    } else {
        None
    }
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long)]
    source: String,

    #[arg(short, long)]
    target: Option<String>,

    #[arg(short, long)]
    covers: bool,

    #[arg(long)]
    creds: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename = "camelCase")]
struct Creds {
    tidal_token_url: Option<String>,
    tidal_client_id: Option<String>,
    tidal_refresh_token: Option<String>,

    tidal_access_token: Option<String>,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let start = SystemTime::now();

    let mut default_headers = header::HeaderMap::new();
    default_headers.insert(
        "Accept",
        header::HeaderValue::from_static("application/json"),
    );
    default_headers.insert(
        "User-Agent",
        header::HeaderValue::from_static("PostmanRuntime/7.33.0"),
    );
    let artwork_client = Client::builder()
        .default_headers(default_headers)
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap();

    let source_dir = args.source;
    let target_dir = args.target;
    let fetch_covers = args.covers;

    let creds = if let Some(path) = args.creds {
        let data = fs::read_to_string(path).expect("Unable to read file");
        let res: Creds = serde_json::from_str(&data).expect("Unable to parse");
        Some(res)
    } else {
        None
    };

    let tidal_access_token = if let Some(creds) = creds {
        Some(if let Some(access_token) = creds.tidal_access_token {
            access_token
        } else if let Some(refresh_token) = creds.tidal_refresh_token {
            let params = [
                (
                    "client_id",
                    creds.tidal_client_id.expect("Missing clientId"),
                ),
                ("grant_type", "refresh_token".to_string()),
                ("refresh_token", refresh_token),
                ("scope", "r_usr w_usr".to_string()),
            ];

            let response = artwork_client
                .post(creds.tidal_token_url.expect("Missing tokenUrl"))
                .form(&params)
                .send()
                .await
                .unwrap()
                .json::<serde_json::Value>()
                .await
                .expect("Failed to get access_token from token url");

            response
                .get("access_token")
                .expect("No access_token on response")
                .as_str()
                .expect("access_token is not a string")
                .to_string()
        } else {
            panic!("Invalid creds file");
        })
    } else {
        None
    };

    let mut updated = Vec::new();

    for path in fs::read_dir(source_dir)
        .unwrap()
        .filter_map(|p| p.ok())
        .filter(|p| p.metadata().unwrap().is_dir())
        .map(|p| p.path())
    {
        let children = path
            .read_dir()
            .unwrap()
            .filter_map(|f| f.ok())
            .map(|f| f.path())
            .collect::<Vec<_>>();

        let album_dirs = if children.clone().into_iter().any(|p| p.is_dir()) {
            children.into_iter().filter(|p| p.is_dir()).collect()
        } else if path.is_dir() {
            vec![path]
        } else {
            vec![]
        };

        for dir in album_dirs {
            if let Some(value) = copy_album_dir_contents(
                target_dir.clone(),
                dir,
                &artwork_client,
                fetch_covers,
                tidal_access_token.clone(),
            )
            .await
            {
                updated.push(value);
            }
        }
    }

    println!("==================================================");

    if !updated.is_empty() {
        println!("Updated following albums:");
        updated.iter().for_each(|p| println!("{}", p));
    } else {
        println!("All up-to-date");
    }
    let end = SystemTime::now();

    println!("Took {}ms", end.duration_since(start).unwrap().as_millis());
}
