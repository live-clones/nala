use std::collections::HashSet;
use std::fs;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use indicatif::{ProgressBar, ProgressStyle};
use ratatui::backend::Backend;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::{
	Block, BorderType, Borders, List, ListItem, ListState, Padding, Paragraph, StatefulWidget,
	Widget,
};
use ratatui::Terminal;
use regex::Regex;
use reqwest::Client;
use rust_apt::new_cache;
use rust_apt::package::Package;
use rust_apt::tagfile::{parse_tagfile, TagSection};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio::time::Duration;

use crate::config::{Config, Paths};
use crate::dprint;
use crate::util::{init_terminal, restore_terminal, sudo_check, NalaRegex};

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
		return Ok(component + " restricted universe multiverse");
	}

	bail!("{distro} is unsupported.")
}

struct FetchItem {
	url: String,
	score: String,
	selected: bool,
}

impl FetchItem {
	fn to_list_items(&self) -> (ListItem, ListItem) {
		let (char, style) =
			if self.selected { ('✓', Color::Cyan) } else { ('☐', Color::White) };
		(
			ListItem::new(Line::styled(format!("{char} {}", self.url), style)),
			ListItem::new(Line::styled(&self.score, style)),
		)
	}
}

struct StatefulList {
	state: ListState,
	items: Vec<FetchItem>,
	align: (usize, usize),
	last_selected: Option<usize>,
}

impl StatefulList {
	fn new(scored: Vec<(String, u128)>) -> StatefulList {
		let mut items = vec![];
		let mut align = 0;
		let mut score_align = 0;
		for (url, u_score) in scored {
			// Calculate alignment
			if url.len() > align {
				align = url.len();
			}
			let score = format!("{u_score} ms");
			if score.len() > score_align {
				score_align = score.len()
			}

			items.push(FetchItem {
				url,
				score,
				selected: false,
			});
		}

		StatefulList {
			state: ListState::default(),
			align: (align, score_align),
			items,
			last_selected: None,
		}
	}

	fn next(&mut self) {
		let i = match self.state.selected() {
			Some(i) => {
				if i >= self.items.len() - 1 {
					0
				} else {
					i + 1
				}
			},
			None => self.last_selected.unwrap_or(0),
		};
		self.state.select(Some(i));
	}

	fn previous(&mut self) {
		let i = match self.state.selected() {
			Some(i) => {
				if i == 0 {
					self.items.len() - 1
				} else {
					i - 1
				}
			},
			None => self.last_selected.unwrap_or(0),
		};
		self.state.select(Some(i));
	}
}

struct FetchTui {
	items: StatefulList,
}

impl FetchTui {
	fn new(scored: Vec<(String, u128)>) -> Self {
		FetchTui {
			items: StatefulList::new(scored),
		}
	}

	/// Changes the status of the selected list item
	fn change_status(&mut self) {
		if let Some(i) = self.items.state.selected() {
			self.items.items[i].selected = match self.items.items[i].selected {
				true => false,
				false => true,
			}
		}
	}

	fn go_top(&mut self) { self.items.state.select(Some(0)); }

	fn go_bottom(&mut self) { self.items.state.select(Some(self.items.items.len() - 1)); }

	fn run(mut self, mut terminal: Terminal<impl Backend>) -> Result<Vec<String>> {
		loop {
			self.draw(&mut terminal)?;

			if let Event::Key(key) = event::read()? {
				if key.kind == KeyEventKind::Press {
					use KeyCode::*;
					match key.code {
						Char('q') | Enter => {
							// Return only the selected Urls.
							return Ok(self
								.items
								.items
								.into_iter()
								.filter(|f| f.selected)
								.map(|f| f.url)
								.collect());
						},
						Char('j') | Down => self.items.next(),
						Char('k') | Up => self.items.previous(),
						Char(' ') => self.change_status(),
						Char('g') | Home => self.go_top(),
						Char('G') | End => self.go_bottom(),
						_ => {},
					}
				}
			}
		}
	}

	fn draw(&mut self, terminal: &mut Terminal<impl Backend>) -> Result<()> {
		terminal.draw(|f| f.render_widget(self, f.size()))?;
		Ok(())
	}

	fn render_lists(&mut self, area: Rect, buf: &mut Buffer) {
		let outer_block = Block::default()
			.title("  Nala Fetch  ".reset().bold())
			.title_alignment(Alignment::Center)
			.add_modifier(Modifier::BOLD)
			.borders(Borders::ALL)
			.border_type(BorderType::Rounded)
			.fg(Color::Cyan);

		let mirror_block = fetch_block("Mirrors:");
		let score_block = fetch_block("Score:");

		let [mirror_area, score_area] = Layout::horizontal([
			Constraint::Length(self.items.align.0 as u16 + 4),
			Constraint::Length(self.items.align.1 as u16),
		])
		.areas(outer_block.inner(area));

		outer_block.render(area, buf);

		let mut mirror_items: Vec<ListItem> = vec![];
		let mut score_items: Vec<ListItem> = vec![];

		for fetch_item in &self.items.items {
			let item = fetch_item.to_list_items();

			mirror_items.push(item.0);

			score_items.push(item.1);
		}

		let mirror_items = item_list(mirror_block, mirror_items);
		let score_items = item_list(score_block, score_items);

		StatefulWidget::render(mirror_items, mirror_area, buf, &mut self.items.state);
		StatefulWidget::render(score_items, score_area, buf, &mut self.items.state);
	}
}

impl Widget for &mut FetchTui {
	fn render(self, area: Rect, buf: &mut Buffer) {
		// Create a space for header, todo list and the footer.
		let [list_area, info_area, footer_area] = Layout::vertical([
			Constraint::Min(0),
			Constraint::Length(2),
			Constraint::Length(2),
		])
		.areas(area);

		self.render_lists(list_area, buf);

		Paragraph::new("\nScore is how many milliseconds it takes to download the Release file.")
			.centered()
			.style(Style::new().italic())
			.render(info_area, buf);

		Paragraph::new(
			"\nUse ↓↑ to move, Space to select/unselect, Home/End to go top/bottom, q/Enter to \
			 exit.",
		)
		.centered()
		.render(footer_area, buf);
	}
}

fn item_list<'a>(block: Block<'a>, item_vec: Vec<ListItem<'a>>) -> List<'a> {
	List::new(item_vec).block(block).highlight_style(
		Style::default()
			.add_modifier(Modifier::BOLD)
			.add_modifier(Modifier::REVERSED)
			.fg(Color::Blue),
	)
}

fn fetch_block(title: &str) -> Block {
	Block::default()
		.title(title)
		.fg(Color::White)
		.padding(Padding::vertical(1))
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

	// Get the current sources on disk to not create duplicates
	let sources = parse_sources(config)?;

	// Get the mirrors
	let mut net_select = fetch_mirrors(config, &countries, &distro)?;

	// Remove domains that are already defined on disk
	let mut remove = HashSet::new();
	for mirror in &net_select {
		for source in &sources {
			if mirror.contains(source) {
				remove.insert(mirror.to_string());
			}
		}
	}
	net_select.retain(|n| !remove.contains(n));

	// Score the mirrors
	let scored = score_handler(config, net_select, &release)?;

	if scored.is_empty() {
		bail!("Nala was unable to find any mirrors.")
	}

	// Only run the TUI if --auto is not on
	let chosen = if config.auto.is_some() {
		scored.into_iter().map(|(s, _)| s).collect()
	} else {
		let terminal = init_terminal()?;
		let chosen = FetchTui::new(scored).run(terminal)?;
		restore_terminal()?;
		chosen
	};

	if chosen.is_empty() {
		bail!("No mirrors were selected.")
	}

	let mut nala_sources = "# Sources file built for nala\n\n".to_string();
	// Types: deb deb-src
	// URIs: https://deb.volian.org/volian/
	// Suites: scar
	// Components: main
	// Signed-By: /usr/share/keyrings/volian-archive-scar-unstable.gpg
	nala_sources += if config.get_bool("sources", false) {
		"Types: deb\n"
	} else {
		"Types: deb deb-src\n"
	};

	nala_sources += "URIs: ";
	for (i, mirror) in chosen.iter().enumerate() {
		if config.auto.is_some_and(|auto| i + 1 > auto as usize) {
			break;
		}
		if i > 0 {
			nala_sources += "      ";
		}
		nala_sources += &format!("{mirror}\n");
	}
	nala_sources += &format!("Suites: {release}\n");
	nala_sources += &format!(
		"Components: {}\n",
		check_non_free(config, &chosen, component, &release)?
	);

	// Hardcode for now
	// let mut file = fs::File::open("/etc/apt/sources.list.d/nala.sources")?;
	// fs::write("/etc/apt/sources.list.d/nala.sources", nala_sources)?;

	println!("{nala_sources}");

	Ok(())
}

fn domain_from_list(regex: &Regex, line: &str) -> Option<String> {
	if line.starts_with('#') || line.is_empty() {
		return None;
	}
	regex_string(regex, line)
}

fn regex_string(regex: &Regex, line: &str) -> Option<String> {
	Some(regex.captures(line)?.get(1)?.as_str().to_string())
}

fn parse_sources(config: &Config) -> Result<HashSet<String>> {
	let regex = crate::util::NalaRegex::new();

	let mut sources = HashSet::new();

	// Read and extract domains from the main sources.list file
	let main = config.get_file(&Paths::SourceList);
	for line in fs::read_to_string(&main)
		.with_context(|| format!("Failed to read {main}"))?
		.lines()
	{
		if let Some(domain) = domain_from_list(regex.domain()?, line) {
			sources.insert(domain);
		}
	}

	// Parts could be either .list or .sources
	let parts = config.get_path(&Paths::SourceParts);
	for file in fs::read_dir(&parts).with_context(|| format!("Failed to read '{parts}'"))? {
		let path = file?.path();
		if path.is_dir() {
			continue;
		}

		let filename = path.to_string_lossy();

		// Don't consider nala-sources as it'll be overwritten
		if filename.contains("nala-sources") {
			continue;
		}

		// Continue if the file isn't .sources or .list
		if !filename.ends_with(".sources") && !filename.ends_with(".list") {
			continue;
		}

		let data = fs::read_to_string(&path)
			.with_context(|| format!("Failed to read '{}'", path.display()))?;

		if filename.ends_with(".sources") {
			for section in parse_tagfile(&data)? {
				let enabled = section.get_default("Enabled", "yes").to_lowercase();

				if ["no", "false", "0"].contains(&enabled.as_str()) {
					continue;
				}

				let Some(uris) = section.get("URIs") else {
					continue;
				};

				for uri in uris.split_whitespace() {
					if uri.is_empty() {
						continue;
					}

					if let Some(domain) = regex_string(regex.domain()?, uri) {
						sources.insert(domain);
					}
				}
			}
			continue;
		}

		if filename.ends_with(".list") {
			for line in data.as_str().lines() {
				if let Some(domain) = domain_from_list(regex.domain()?, line) {
					sources.insert(domain);
				}
			}
		}
	}
	Ok(sources)
}

fn fetch_mirrors(
	config: &Config,
	countries: &Option<HashSet<String>>,
	distro: &str,
) -> Result<HashSet<String>> {
	let mut net_select = HashSet::new();
	if distro == "debian" {
		let response =
			reqwest::blocking::get("https://mirror-master.debian.org/status/Mirrors.masterlist")?
				.text()?;

		let tagfile = rust_apt::tagfile::parse_tagfile(&response).unwrap();
		let arches = config.apt.get_architectures();

		for section in tagfile {
			if let Some(url) = debian_url(countries, &section, &arches) {
				net_select.insert(url);
			}
		}
	} else if distro == "ubuntu" {
		let response =
			reqwest::blocking::get("https://launchpad.net/ubuntu/+archivemirrors-rss")?.text()?;

		let regex = NalaRegex::new();
		let mirrors = response.split("<item>");
		for mirror in mirrors {
			if let Some(url) = ubuntu_url(config, countries, &regex, mirror) {
				net_select.insert(url);
			}
		}
	} else if distro == "devuan" {
		let response =
			reqwest::blocking::get("https://pkgmaster.devuan.org/mirror_list.txt")?.text()?;

		let tagfile = rust_apt::tagfile::parse_tagfile(&response).unwrap();
		for section in tagfile {
			if let Some(url) = devuan_url(countries, &section) {
				net_select.insert(url);
			}
		}
	}
	Ok(net_select)
}

#[tokio::main]
async fn check_non_free(
	config: &Config,
	chosen: &[String],
	mut component: String,
	release: &str,
) -> Result<String> {
	let mut set = JoinSet::new();

	if !config.get_bool("non_free", false) {
		return Ok(component);
	}

	let client = Client::builder().timeout(Duration::from_secs(5)).build()?;

	for url in chosen.iter() {
		set.spawn(
			client
				.get(format!("{url}/dists/{release}/non-free-firmware/"))
				.send(),
		);
	}

	let mut values = Vec::with_capacity(set.len());
	// Run all of the futures.
	while let Some(res) = set.join_next().await {
		values.push(res.is_ok_and(|r| r.is_ok_and(|r| r.error_for_status().is_ok())));
	}

	// Debatable that we should add separate entries if it exists or not
	if values.iter().all(|b| *b) {
		component += " non-free-firmware";
		return Ok(component);
	}
	Ok(component)
}

#[tokio::main]
/// Score the mirrors and provide a progress bar.
async fn score_handler(
	config: &Config,
	mirror_strings: HashSet<String>,
	release: &str,
) -> Result<Vec<(String, u128)>> {
	let mut set = JoinSet::new();

	let semp = Arc::new(Semaphore::new(30));
	let mut score = vec![];

	// Setup Progress Bar
	let pb = ProgressBar::new(mirror_strings.len() as u64);
	pb.set_style(
		ProgressStyle::with_template("{prefix:.bold}[{bar:40.cyan/red}] {percent}% • {pos}/{len}")
			.unwrap()
			.progress_chars("━━"),
	);
	pb.set_prefix("Testing Mirrors: ");

	for url in &mirror_strings {
		set.spawn(net_select_score(
			semp.clone(),
			config.get_bool("https_only", false),
			url.strip_suffix('/').unwrap_or(url).to_string(),
			release.to_string(),
		));
	}

	// Run all of the futures.
	while let Some(res) = set.join_next().await {
		if let Ok(Ok(response)) = res {
			score.push(response)
		}
		pb.inc(1);
	}

	// Move FetchScore out of its Arc and then return the final vec.
	score.sort_by_key(|k| k.1);
	Ok(score)
}

/// Score the url with https and http depending on config.
async fn net_select_score(
	semp: Arc<Semaphore>,
	https_only: bool,
	url: String,
	release: String,
) -> Result<(String, u128)> {
	let sem = semp.acquire_owned().await?;
	let https = url.replace("http://", "https://");
	let client = Client::builder().timeout(Duration::from_secs(5)).build()?;

	if let Ok(response) = fetch_release(&client, &https, &release).await {
		return Ok(response);
	}

	// We don't need to check http if it's https-only or if its only https
	if !https_only && !url.contains("https://") {
		if let Ok(response) = fetch_release(&client, &url, &release).await {
			return Ok(response);
		}
	}

	drop(sem);
	bail!("Could not get to '{url}'")
}

/// Fetch the release file and handle errors
///
/// This will return Some(String) if its NOT successful
/// None is successful
async fn fetch_release(client: &Client, base_url: &str, release: &str) -> Result<(String, u128)> {
	// TODO: Should we verify the release file is proper?
	let before = std::time::Instant::now();
	client
		.get(format!("{base_url}/dists/{release}/Release"))
		.send()
		.await?
		.error_for_status()?;
	let after = before.elapsed().as_millis();
	Ok((base_url.to_string(), after))
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
