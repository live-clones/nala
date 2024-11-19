use std::fs::File;
use std::io::Read;

use anyhow::{bail, Ok, Result};
use ar::Archive;
use rust_apt::tagfile;
use tar::Archive as Tarchive;
use xz2::read::XzDecoder;
// pub fn read_archive() {
// 	let file =
// File::open("/var/cache/apt/archives/volian-archive-nala_0.3.1_all.deb").
// unwrap(); 	let mut a = Archive::new(file);

// 	for file in a.entries().unwrap() {
// 		// Make sure there wasn't an I/O error
// 		let mut file = file.unwrap();

// 		// Inspect metadata about the file
// 		println!("{:?}", file.header().path().unwrap());
// 		println!("{}", file.header().size().unwrap());

// 		// files implement the Read trait
// 		let mut s = String::new();
// 		file.read_to_string(&mut s).unwrap();
// 		println!("{}", s);
// 	}
// }

#[derive(Debug)]
pub struct DebFile<'a> {
	pub path: &'a str,
	tag: tagfile::TagSection,
}

impl<'a> DebFile<'a> {
	pub fn new(path: &'a str) -> Result<Self> {
		let tag = read_archive(path)?;

		if tag
			.get("Package")
			.is_some_and(|_| tag.get("Version").is_some())
		{
			return Ok(DebFile { path, tag });
		}
		bail!("Not a valid DebFile")
	}

	pub fn name(&self) -> &str { self.tag.get("Package").unwrap() }

	pub fn version(&self) -> &str { self.tag.get("Version").unwrap() }
}

pub fn read_archive(path: &str) -> Result<tagfile::TagSection> {
	// Read an archive from the file foo.a:
	let mut archive = Archive::new(File::open(path).unwrap());

	// Iterate over all entries in the archive:
	while let Some(entry_result) = archive.next_entry() {
		let entry = entry_result.unwrap();
		let filename = std::str::from_utf8(entry.header().identifier()).unwrap();

		if !filename.contains("control.tar") {
			continue;
		}
		let decompress = XzDecoder::new(entry);
		let mut tarchive = Tarchive::new(decompress);

		for file in tarchive.entries().unwrap() {
			let mut entry = file.unwrap();

			if entry.header().path().unwrap().as_os_str() == "./control" {
				let mut control = String::new();
				entry.read_to_string(&mut control).unwrap();

				// Move this into DebFile::new?
				return Ok(tagfile::parse_tagfile(&control)
					.unwrap()
					.into_iter()
					.next()
					.unwrap());
			}
		}
	}

	bail!("Could not determine valid deb file")
}
