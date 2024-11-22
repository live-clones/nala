use std::fs::File;
use std::io::Read;
use std::path::Path;

use anyhow::{bail, Ok, Result};
use ar::Archive;
use rust_apt::tagfile;
use tar::Archive as Tarchive;
use xz2::read::XzDecoder;

#[derive(Debug)]
pub struct DebFile<'a> {
	pub path: &'a str,
	tag: tagfile::TagSection,
}

impl<'a> DebFile<'a> {
	pub fn new<P: AsRef<Path>>(path: &'a P) -> Result<Self> {
		let tag = read_archive(path)?;

		if tag
			.get("Package")
			.is_some_and(|_| tag.get("Version").is_some())
		{
			return Ok(DebFile {
				path: path.as_ref().to_str().unwrap(),
				tag,
			});
		}
		bail!("Not a valid DebFile")
	}

	pub fn name(&self) -> &str { self.tag.get("Package").unwrap() }

	pub fn version(&self) -> &str { self.tag.get("Version").unwrap() }
}

pub fn read_archive<P: AsRef<Path>>(path: P) -> Result<tagfile::TagSection> {
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
