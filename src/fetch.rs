use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{bail, Result};
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::Client;
use rust_apt::new_cache;
use rust_apt::package::Package;
use rust_apt::tagfile::TagSection;
use tokio::sync::{Mutex, Semaphore};
use tokio::task::JoinSet;
use tokio::time::Duration;

use crate::config::Config;
use crate::dprint;
use crate::util::{sudo_check, NalaRegex};

type ScoreVec = Arc<Mutex<Vec<(String, u128)>>>;

fn get_origin_codename(pkg: Option<Package>) -> Option<(String, String)> {
	let pkg_file = pkg?.candidate()?.package_files().next()?;

	if let (Ok(origin), Ok(codename)) = (pkg_file.origin(), pkg_file.codename()) {
		return Some((origin.to_string(), codename.to_string()));
	}
	None
}

fn detect_release(config: &Config) -> Result<(String, String)> {
	for distro in ["debian", "ubuntu", "devuan"] {
		if let Some(value) = config.string_map.get(distro) {
			dprint!(config, "Distro '{distro} {value}' passed on CLI");
			return Ok((distro.to_string(), value.to_lowercase()));
		}
	}

	let cache = new_cache!()?;

	for keyring in [
		"devuan-keyring",
		"debian-archive-keyring",
		"ubuntu-keyring",
		"apt",
	] {
		if let Some((origin, codename)) = get_origin_codename(cache.get(keyring)) {
			dprint!(config, "Distro/Release Found on '{keyring}'");
			return Ok((origin.to_lowercase(), codename.to_lowercase()));
		}
	}
	bail!("There was an issue detecting release.");
}

fn get_component(config: &Config, distro: &str) -> Result<String> {
	let mut component = "main".to_string();
	if distro == "devuan" || distro == "debian" {
		if config.get_bool("non_free", false) {
			component += " contrib non-free"
		}
		return Ok(component);
	}

	if distro == "ubuntu" {
		// It's Ubuntu, you probably don't care about foss
		return Ok(component);
	}

	bail!("{distro} is unsupported.")
}

pub fn fetch(config: &Config) -> Result<()> {
	sudo_check(config)?;

	let (distro, release) = detect_release(config)?;

	dprint!(config, "Detected {distro}:{release}");

	let component = get_component(config, &distro)?;

	let countries: Option<HashSet<String>> = match config.countries() {
		Some(values) => {
			let mut hash_set = HashSet::new();
			for value in values {
				hash_set.insert(value.to_uppercase());
			}
			Some(hash_set)
		},
		None => None,
	};

	let mut net_select = HashSet::new();

	// Fetch the mirrors
	if distro == "debian" {
		let response =
			reqwest::blocking::get("https://mirror-master.debian.org/status/Mirrors.masterlist")?
				.text()?;

		let tagfile = rust_apt::tagfile::parse_tagfile(&response).unwrap();
		let arches = config.apt.get_architectures();

		for section in tagfile {
			if let Some(url) = debian_url(&countries, &section, &arches) {
				net_select.insert(url);
			}
		}
	} else if distro == "ubuntu" {
		let response =
			reqwest::blocking::get("https://launchpad.net/ubuntu/+archivemirrors-rss")?.text()?;

		let regex = NalaRegex::new();
		let mirrors = response.split("<item>");
		for mirror in mirrors {
			if let Some(url) = ubuntu_url(config, &countries, &regex, mirror) {
				net_select.insert(url);
			}
		}
	} else if distro == "devuan" {
		let response =
			reqwest::blocking::get("https://pkgmaster.devuan.org/mirror_list.txt")?.text()?;

		let tagfile = rust_apt::tagfile::parse_tagfile(&response).unwrap();
		for section in tagfile {
			if let Some(url) = devuan_url(&countries, &section) {
				net_select.insert(url);
			}
		}
	}

	// Get the Vec out of Arc<Mutex<T>>
	let scored = Arc::into_inner(score_handler(config, net_select, &release)?)
		.expect("Nothing should be locked")
		.into_inner();

	if scored.is_empty() {
		bail!("Nala was unable to find any mirrors.")
	}

	for (i, (url, score)) in scored.iter().enumerate() {
		if i > 9 {
			break;
		}
		println!("{url} {score}")
	}

	Ok(())
}


#[tokio::main]
async fn score_handler(
	config: &Config,
	net_select: HashSet<String>,
	release: &str,
) -> Result<ScoreVec> {
	let scored = Arc::new(Mutex::new(Vec::new()));
	let semp = Arc::new(Semaphore::new(30));
	let mut set = JoinSet::new();

	let pb = Arc::new(ProgressBar::new(net_select.len() as u64));
	pb.set_style(
		ProgressStyle::with_template("{prefix:.bold}[{bar:40.cyan/red}] {percent}% • {pos}/{len}")
			.unwrap()
			.progress_chars("━━"),
	);
	pb.set_prefix("Testing Mirrors: ");



	for url in &net_select {
		set.spawn(net_select_score(
			pb.clone(),
			config.debug(),
			config.get_bool("https_only", false),
			scored.clone(),
			semp.clone(),
			format!(
				"{}/dists/{release}/Release",
				url.strip_suffix('/').unwrap_or(url)
			),
		));
	}

	// Run all of the futures.
	while let Some(res) = set.join_next().await {
		res??;
	}

	scored.lock().await.sort_by_key(|k| k.1);
	Ok(scored)
}

async fn fetch_release(client: &Client, scored: &ScoreVec, url: &str, debug: bool) -> bool {
	let before = std::time::Instant::now();
	// Ignores errors and just doesn't add the url into the vec
	// Debug print the error messages
	match client.get(url).send().await {
		Ok(response) => {
			if let Err(e) = response.error_for_status() {
				if debug {
					eprintln!("DEBUG: {e}");
				}
				return false;
			}
		},
		Err(e) => {
			if debug {
				eprintln!("DEBUG: {e}");
			}
			return false;
		},
	};
	let after = before.elapsed().as_millis();
	scored.lock().await.push((url.to_string(), after));
	true
}

async fn net_select_score(
	pb: Arc<ProgressBar>,
	debug: bool,
	https_only: bool,
	scored: ScoreVec,
	semp: Arc<Semaphore>,
	url: String,
) -> Result<()> {
	let sem = semp.acquire_owned().await?;
	let client = Client::builder().timeout(Duration::from_secs(5)).build()?;

	let https = url.replace("http://", "https://");

	if fetch_release(&client, &scored, &https, debug).await {
		pb.inc(1);
		return Ok(());
	}

	if !https_only {
		fetch_release(&client, &scored, &url, debug).await;
	}
	drop(sem);
	pb.inc(1);
	Ok(())
}

fn debian_url(
	countries: &Option<HashSet<String>>,
	section: &TagSection,
	arches: &[String],
) -> Option<String> {
	// If there are countries provided
	if let Some(hash_set) = countries {
		let country = section.get("Country")?.split_whitespace().next()?;

		// If it doesn't match any provided return None
		if !hash_set.contains(country) {
			return None;
		}
	}

	// There were either no countries provided or there was a match
	let mirror_arches = section.get("Archive-architecture")?;
	if arches.iter().all(|arch| mirror_arches.contains(arch)) {
		return Some(format!(
			"http://{}{}",
			section.get("Site")?,
			section.get("Archive-http")?
		));
	}
	None
}

fn ubuntu_url(
	config: &Config,
	countries: &Option<HashSet<String>>,
	regex: &NalaRegex,
	mirror: &str,
) -> Option<String> {
	if mirror.contains("<title>Ubuntu Archive Mirrors Status</title>") {
		return None;
	}

	let only_ports = config
		.apt
		.get_architectures()
		.iter()
		.any(|arch| arch != "amd64" && arch != "i386");

	if let Some(hash_set) = countries {
		if !hash_set.contains(
			regex
				.ubuntu_country()
				.unwrap()
				.captures(mirror)?
				.get(1)?
				.as_str(),
		) {
			return None;
		}
	}

	let url = regex
		.ubuntu_url()
		.unwrap()
		.captures(mirror)?
		.get(1)?
		.as_str();
	let is_ports = url.contains("ubuntu-ports");

	// Don't return non ports if we only want ports
	if only_ports && !is_ports {
		return None;
	}

	// Don't return ports if we don't want only_ports
	if !only_ports && is_ports {
		return None;
	}

	Some(url.to_string())
}

fn devuan_url(countries: &Option<HashSet<String>>, section: &TagSection) -> Option<String> {
	if !section.get("Protocols")?.contains("HTTP") {
		return None;
	}

	if let Some(hash_set) = countries {
		for country in hash_set {
			if !section.get("CountryCode")?.contains(country) {
				return None;
			}
		}
	}

	Some(format!("http://{}/devuan", section.get("BaseURL")?.trim()))
}
