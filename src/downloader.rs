use std::cmp::min;
use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::fs;
use std::io::Read;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use bytes::{BufMut, Bytes, BytesMut};
use indicatif::{ProgressBar, ProgressState, ProgressStyle};
use once_cell::sync::{Lazy, OnceCell};
use regex::{Regex, RegexBuilder};
use reqwest::{self, Client, Response};
use rust_apt::cache::Cache;
use rust_apt::new_cache;
use rust_apt::package::{Package, Version};
use rust_apt::records::RecordField;
use tokio::task::JoinSet;

use crate::config::Config;

pub struct MirrorRegex {
	mirror: OnceCell<Regex>,
	mirror_file: OnceCell<Regex>,
}

impl MirrorRegex {
	fn new() -> Self {
		MirrorRegex {
			mirror: OnceCell::new(),
			mirror_file: OnceCell::new(),
		}
	}

	fn mirror(&self) -> Result<&Regex> {
		self.mirror.get_or_try_init(|| {
			Ok(RegexBuilder::new(r"mirror://(.*?/.*?)/")
				.case_insensitive(true)
				.build()?)
		})
	}

	fn mirror_file(&self) -> Result<&Regex> {
		self.mirror_file.get_or_try_init(|| {
			Ok(RegexBuilder::new(r"mirror\+file:(/.*?)/pool")
				.case_insensitive(true)
				.build()?)
		})
	}
}

// #[derive(Clone, Debug)]
pub struct URI {
	// version: &'a Version<'a>,
	uris: HashSet<String>,
	size: u64,
	path: String,
	hash_type: String,
	filename: String,
}

impl URI {
	async fn from_version<'a>(
		version: &'a Version<'a>,
		// 		package: &'a Package<'a>,
		cache: &Cache,
		config: &Config,
		downloader: &mut Downloader,
	) -> Result<Arc<URI>> {
		let filename = version
			.get_record(RecordField::Filename)
			.unwrap()
			.split_terminator("/")
			.last()
			.unwrap()
			.to_string();
		println!("filename: {filename}");
		Ok(Arc::new(URI {
			uris: filter_uris(version, cache, config, downloader).await?,
			size: 0,
			path: "".to_string(),
			hash_type: "".to_string(),
			filename,
		}))
	}
}

pub struct Progress {
	progress: ProgressBar,
	data: u64,
}

impl Progress {
	fn new(total: u64) -> Self {
		Progress {
			progress: ProgressBar::new(total),
			data: 0,
		}
	}
}

pub struct Downloader {
	client: Client,
	uri_list: Vec<Arc<URI>>,
	untrusted: HashSet<String>,
	not_found: Vec<String>,
	mirrors: HashMap<String, String>,
	mirror_regex: MirrorRegex,
	progress: ProgressBar,
	data: u64,
}

impl Downloader {
	fn new() -> Self {
		Downloader {
			client: reqwest::Client::new(),
			uri_list: vec![],
			untrusted: HashSet::new(),
			not_found: vec![],
			mirrors: HashMap::new(),
			mirror_regex: MirrorRegex::new(),
			progress: ProgressBar::new(0),
			data: 0,
		}
	}

	fn start_progress(&self) {
		let mut total: u64 = 0;
		for uri in &self.uri_list {
			total += uri.size;
		}
		self.progress.set_length(total);

		let mut message = String::new();
		message += "\n";
		message += "│  Total Packages: 81/391\n";
		message += "│  Last Completed: libreoffice-gtk3_4%3a7.5.7-1_amd64.deb\n";
		message += "│  [{eta}] [{wide_bar:.cyan/red}] {bytes}/{total_bytes}";

		self.progress.set_style(
			ProgressStyle::with_template(&message)
				.unwrap()
				.with_key("eta", |state: &ProgressState, w: &mut dyn Write| {
					write!(w, "{:.1}", state.eta().as_secs_f64()).unwrap()
				})
				.progress_chars("━━━"),
		);
	}

	// async fn start_downloads(&mut self) -> Result<()> {
	// 	let mut set = JoinSet::new();

	// 	for uri in &self.uri_list {
	// 		set.spawn(self.download_file(uri.clone()));
	// 	}

	// 	while let Some(res) = set.join_next().await {
	// 		let _out = res??;
	// 	}
	// 	drop(set);
	// 	Ok(())
	// }

	async fn download_file(&mut self, uri: Arc<URI>) -> Result<()> {
		let mut response = self
			.client
			.get(uri.uris.iter().next().unwrap())
			.send()
			.await?;
		while let Some(chunk) = response.chunk().await? {
			self.data += chunk.len() as u64;

			// let new = min(downloaded + 223211, total_size);
			// downloaded = new;
			self.progress.set_position(self.data);
		}
		Ok(())
	}
}

pub fn mirror_filter<'a>(
	version: &'a Version<'a>,
	mirrors: &mut HashMap<String, String>,
	uris: &mut HashSet<String>,
	filename: &str,
) -> Result<bool> {
	if let Some(data) = mirrors.get(filename) {
		for line in data.lines() {
			if !line.is_empty() && !line.starts_with("#") {
				uris.insert(
					line.to_string() + "/" + &version.get_record(RecordField::Filename).unwrap(),
				);
			}
		}
		return Ok(true);
	}
	Ok(false)
}

pub fn uri_trusted<'a>(cache: &Cache, version: &'a Version<'a>, uri: &str) -> Result<bool> {
	for mut package_file in version.package_files() {
		let archive = package_file.archive()?;

		if uri.contains(package_file.site()?) && archive != "now" {
			return Ok(cache.is_trusted(&mut package_file));
		}
	}
	Ok(false)
}

pub async fn filter_uris<'a>(
	version: &'a Version<'a>,
	cache: &Cache,
	config: &Config,
	downloader: &mut Downloader,
) -> Result<HashSet<String>> {
	let mut filtered = HashSet::new();

	for uri in version.uris() {
		// Sending a file path through the downloader will cause it to lock up
		// These have already been handled before the downloader runs.
		if uri.starts_with("file:") {
			continue;
		}

		if !uri_trusted(cache, version, &uri)? {
			downloader
				.untrusted
				.insert(config.color.red(version.parent().name()).to_string());
		}

		if uri.starts_with("mirror+file:") {
			if let Some(file_match) = downloader.mirror_regex.mirror_file()?.captures(&uri) {
				let filename = file_match.get(1).unwrap().as_str();
				if !downloader.mirrors.contains_key(filename) {
					downloader.mirrors.insert(
						filename.to_string(),
						fs::read_to_string(filename).with_context(|| {
							format!("Failed to read {filename}, using defaults")
						})?,
					);
				};

				if mirror_filter(version, &mut downloader.mirrors, &mut filtered, filename)? {
					continue;
				}
			}
		}

		if uri.starts_with("mirror:") {
			// Do some things or smth
			if let Some(file_match) = downloader.mirror_regex.mirror()?.captures(&uri) {
				let filename = file_match.get(1).unwrap().as_str();
				if !downloader.mirrors.contains_key(filename) {
					downloader.mirrors.insert(
						filename.to_string(),
						reqwest::get("http://".to_string() + filename)
							.await?
							.text()
							.await?,
					);
				}

				if mirror_filter(version, &mut downloader.mirrors, &mut filtered, filename)? {
					continue;
				}
			}
		}

		// If none of the conditions meet then we just add it to the uris
		filtered.insert(uri);
	}
	Ok(filtered)
}

pub fn untrusted_error(config: &Config, untrusted: &HashSet<String>) -> Result<()> {
	config
		.color
		.warn("The Following packages cannot be authenticated!");

	eprintln!(
		"  {}",
		untrusted
			.iter()
			.map(|s| s.to_string())
			.collect::<Vec<String>>()
			.join(", ")
	);

	if !config.apt.bool("APT::Get::AllowUnauthenticated", false) {
		bail!("Some packages were unable to be authenticated.")
	}

	config
		.color
		.notice("Configuration is set to allow installation of unauthenticated packages.");
	Ok(())
}

#[tokio::main]
pub async fn download(config: &Config) -> Result<()> {
	let mut downloader = Downloader::new();

	if let Some(pkg_names) = config.pkg_names() {
		let cache = new_cache!()?;
		for name in pkg_names {
			if let Some(pkg) = cache.get(name) {
				// This is a bug fix it. It resets the whole list. Is this supposed to be a list
				// of a list?
				let versions: Vec<Version> = pkg.versions().collect();
				for version in &versions {
					if version.is_downloadable() {
						let uri =
							URI::from_version(version, &cache, config, &mut downloader).await?;
						downloader.uri_list.push(uri);
						break;
					}
					// Version wasn't downloadable
					config.color.warn(&format!(
						"Can't find a source to download version '{}' of '{}'",
						version.version(),
						pkg.fullname(false)
					));
				}
				println!("{}", downloader.uri_list.len())
			} else {
				downloader
					.not_found
					.push(config.color.yellow(name).to_string());
			}
		}
	} else {
		bail!("You must specify a package")
	};

	if !downloader.not_found.is_empty() {
		for pkg in &downloader.not_found {
			config.color.error(&format!("{pkg} not found"))
		}
		bail!("Some packages were not found.");
	}

	if !downloader.untrusted.is_empty() {
		untrusted_error(config, &downloader.untrusted)?
	}

	let mut set = JoinSet::new();

	for uri in downloader.uri_list {
		set.spawn(download_file(uri.clone()));
	}

	while let Some(res) = set.join_next().await {
		let _out = res??;
	}

	Ok(())
}

pub async fn download_file(uri: Arc<URI>) -> Result<()> {
	let client = reqwest::Client::new();
	let mut response = client.get(uri.uris.iter().next().unwrap()).send().await?;
	let total = response
		.headers()
		.get("content-length")
		.unwrap()
		.to_str()?
		.parse::<u64>()?;

	let pb = ProgressBar::new(total);
	pb.set_length(total);

	let mut message = String::new();
	message += "\n";
	message += "│  Total Packages: 81/391\n";
	message += "│  Last Completed: libreoffice-gtk3_4%3a7.5.7-1_amd64.deb\n";
	message += "│  [{eta}] [{wide_bar:.cyan/red}] {bytes}/{total_bytes}";

	pb.set_style(
		ProgressStyle::with_template(&message)
			.unwrap()
			.with_key("eta", |state: &ProgressState, w: &mut dyn Write| {
				write!(w, "{:.1}", state.eta().as_secs_f64()).unwrap()
			})
			.progress_chars("━━━"),
	);
	// state.eta().as_secs_f64()
	// let data: BytesMut = BytesMut::new();
	let mut data: u64 = 0;
	// let data: Bytes = Bytes::new;
	while let Some(chunk) = response.chunk().await? {
		data += chunk.len() as u64;

		// let new = min(downloaded + 223211, total_size);
		// downloaded = new;
		pb.set_position(data);
	}
	pb.finish_with_message("downloaded");
	println!("{data} = {total}");

	// let res = reqwest::get(uri.uris.first().unwrap()).await?;
	// dbg!(res.headers());
	// let end = res.bytes().await?.len();
	// println!("{end}");
	Ok(())
}
