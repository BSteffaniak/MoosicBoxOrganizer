use std::{
    env,
    fs::{self},
    path::Path,
};

use audiotags::Tag;
use fs_extra::dir::CopyOptions;

fn main() {
    let args: Vec<String> = env::args().collect();

    let source_dir = if args.len() > 1 {
        args[1].clone()
    } else {
        panic!("Must pass a source_dir argument");
    };
    let target_dir = if args.len() > 2 {
        args[2].clone()
    } else {
        panic!("Must pass a target_dir argument");
    };

    let copy_options = CopyOptions::new();

    let updated = fs::read_dir(source_dir)
        .unwrap()
        .filter_map(|p| p.ok())
        .filter(|p| p.metadata().unwrap().is_dir())
        .map(|p| p.path())
        .filter_map(|path| {
            let flac = fs::read_dir(path.clone())
                .unwrap()
                .filter_map(|p| p.ok())
                .find(|p| p.file_name().to_str().unwrap().ends_with(".flac"))
                .unwrap();

            let tag = Tag::new()
                .read_from_path(flac.path().to_str().unwrap())
                .unwrap();

            let title = tag.title().unwrap();
            let album = tag.album_title().unwrap_or("(none)");
            let artist = tag.artist().or(tag.album_artist()).unwrap();
            let album_dir_name = path.file_name().unwrap().to_str().unwrap();

            println!("====== {} ======", path.clone().to_str().unwrap());
            println!("title: {}", title);
            println!("album title: {}", album);
            println!("album directory name: {}", album_dir_name);
            println!("artist: {}", artist);

            let artist_dir = Path::new(&target_dir).join(artist);

            if !artist_dir.is_dir() {
                println!("Creating artist dir {}", artist_dir.to_str().unwrap());
                let _ = fs::create_dir(artist_dir.clone());
            }

            let album_dir = artist_dir.join(album_dir_name);

            if !album_dir.is_dir() {
                let source = path.to_str().unwrap().clone();
                let target = artist_dir.to_str().unwrap().clone();
                println!("Copying album dir {} -> {}", source, target);
                let _ = fs_extra::dir::copy(source, target, &copy_options);
                Some(String::from(album_dir.to_str().unwrap()))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    println!("==================================================");

    if !updated.is_empty() {
        println!("Updated following albums:");
        updated.iter().for_each(|p| println!("{}", p));
    } else {
        println!("All up-to-date");
    }
}
