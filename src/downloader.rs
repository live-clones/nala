use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::fs;
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context, Result};
use indicatif::{MultiProgress, ProgressBar, ProgressState, ProgressStyle};
use once_cell::sync::{OnceCell, Lazy};
use regex::{Regex, RegexBuilder};
use reqwest;
use rust_apt::cache::Cache;
use rust_apt::new_cache;
use rust_apt::package::Version;
use rust_apt::records::RecordField;
use rust_apt::util::terminal_width;
use tokio::task::JoinSet;

use crate::config::Config;

static HASHMAP: Lazy<HashMap<&str, &str>> = Lazy::new(|| {
	println!("initializing");
	HashMap::from([
		("left_border", "│  "),
		("right_border", "  │"),
		("border", "─"),
		("corner", "╭╮╰╯"),
		("message", "{msg}"),

		("red", "testing"),
	])
});

pub fn build_progress(config: &Config) -> String {
	let left_border = "│  ";
	let right_border = "  │";
	let border = "─";
	let corner = "╭╮╰╯";

	//let message = "{msg}";
	let time_remaining = "";


	let downloading = "Total:".to_string();
	let total_time = "ETA:".to_string();

	let download_fill = " ".repeat(terminal_width() - 6 - downloading.len() - 4);
	let total_fill = " ".repeat(terminal_width() - 6 - total_time.len() - 9);

	let mut message = String::new();
	message += &format!("\n{} {{msg}}, {} {{eta_precise}} ", config.color.package(&downloading), config.color.package(&total_time));
	message += "{wide_bar:.cyan/red} {percent}% • {bytes}/{total_bytes} • {binary_bytes_per_sec}";
	message += &format!("\n╭{}╮", "─".repeat(terminal_width() - 2));
	// message += &format!("\n{left_border}{} {{msg}}{download_fill}{right_border}", config.color.package(&downloading));
	// message += &format!("\n{left_border}{} {{eta_precise}}{total_fill}{right_border}", config.color.package(&total_time));
	//message += "\n│  [{wide_bar:.cyan/red}] {percent}% • {bytes}/{total_bytes} • {binary_bytes_per_sec}  │";
	message += &format!("\n{left_border}{}{right_border}", " ".repeat(terminal_width() - 6));

	message
}


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
		cache: &Cache,
		config: &Config,
		downloader: &mut Downloader,
	) -> Result<Arc<URI>> {
		Ok(Arc::new(URI {
			uris: filter_uris(version, cache, config, downloader).await?,
			size: version.size(),
			path: "".to_string(),
			hash_type: "".to_string(),
			filename: version
				.get_record(RecordField::Filename)
				.expect("Record does not contain a filename!")
				.split_terminator("/")
				.last()
				.expect("Filename is malformed!")
				.to_string(),
		}))
	}
}

// pub struct ProgressString {
// 	left_border: String,
// 	right_border: String,
// 	message: String,
// 	bar: String,
// 	percent: String,
// 	bytes: Bu
// }

pub struct Progress {
	multi: MultiProgress,
	progress: ProgressBar,
	pkgs_downloaded: u64,
	total_pkgs: u64,
	data: u64,
	last_progress: ProgressBar,
}

impl Progress {
	fn new(total: u64, total_pkgs: u64, message: String) -> Arc<Mutex<Self>> {
		let multi = MultiProgress::new();

		let progress = multi.add(ProgressBar::new(total));
		progress.set_style(
			ProgressStyle::with_template(
				&message
			)
			.unwrap()
			.progress_chars("━━━"),
		);

		let last_progress = multi.add(ProgressBar::new(total));
		last_progress.set_style(
			ProgressStyle::with_template(
				&format!("╰{}╯", "─".repeat(terminal_width() - 2)),
			)
			.unwrap()
			.progress_chars("━━━"),
		);
		last_progress.inc(1);

		progress.set_message(format!("{}/{}", 0, total_pkgs));
		Arc::new(Mutex::new(Progress {
			multi,
			progress,
			pkgs_downloaded: 0,
			total_pkgs,
			data: 0,
			last_progress,
		}))
	}
}

pub struct Downloader {
	uri_list: Vec<Arc<URI>>,
	untrusted: HashSet<String>,
	not_found: Vec<String>,
	mirrors: HashMap<String, String>,
	mirror_regex: MirrorRegex,
}

impl Downloader {
	fn new() -> Self {
		Downloader {
			uri_list: vec![],
			untrusted: HashSet::new(),
			not_found: vec![],
			mirrors: HashMap::new(),
			mirror_regex: MirrorRegex::new(),
		}
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

	// HASHMAP.get("nothing").is_none();
	// dbg!(&HASHMAP);
	// print!("WoW!");
	// panic!();

	if let Some(pkg_names) = config.pkg_names() {
		let cache = new_cache!()?;
		for name in pkg_names {
			if let Some(pkg) = cache.get(name) {
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

	let progress = Progress::new(
		downloader.uri_list.iter().map(|uri| uri.size).sum(),
		downloader.uri_list.len() as u64,
		build_progress(config),
	);

	let mut set = JoinSet::new();
	for uri in downloader.uri_list {
		let pkg_name = config.color.package(&uri.filename).to_string();
		set.spawn(download_file(progress.clone(), uri.clone(), pkg_name));
	}

	while let Some(res) = set.join_next().await {
		let _out = res??;
	}

	Ok(())
}

pub async fn download_file(
	progress: Arc<Mutex<Progress>>,
	uri: Arc<URI>,
	pkg_name: String,
) -> Result<()> {
	let client = reqwest::Client::new();
	let mut response = client.get(uri.uris.iter().next().unwrap()).send().await?;

	// ProgressBar::new(uri.size));
	// let pb = progress
	// 	.lock()
	// 	.unwrap()
	// 	.multi
	// 	.add(ProgressBar::new(uri.size));
	let pb = progress.lock().unwrap().multi.insert_from_back(1, ProgressBar::new(uri.size));
	pb.set_style(
		ProgressStyle::with_template(
			"│  {msg} [{wide_bar:.cyan/red}] {percent}% • {bytes}/{total_bytes} • \
			 {binary_bytes_per_sec}  │",
		)
		.unwrap()
		.progress_chars("━━━"),
	);

	pb.set_message(pkg_name);

	while let Some(chunk) = response.chunk().await? {
		progress.lock().unwrap().progress.inc(chunk.len() as u64);
		pb.inc(chunk.len() as u64);
	}
	pb.finish();

	let mut total_pb = progress.lock().unwrap();
	total_pb.pkgs_downloaded += 1;
	total_pb.progress.set_message(format!(
		"{}/{}",
		total_pb.pkgs_downloaded, total_pb.total_pkgs
	));

	Ok(())
}
