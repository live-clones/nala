use std::collections::{HashMap, HashSet};
use std::fmt::format;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode};
use crossterm::execute;
use crossterm::terminal::{
	disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use indicatif::{BinaryBytes, FormattedDuration, HumanBytes, ProgressBar};
use once_cell::sync::OnceCell;
use ratatui::prelude::*;
use ratatui::style::Stylize;
use ratatui::widgets::*;
use regex::{Regex, RegexBuilder};
use reqwest;
use rust_apt::cache::Cache;
use rust_apt::new_cache;
use rust_apt::package::Version;
use rust_apt::records::RecordField;
use rust_apt::util::{terminal_width, unit_str, NumSys};
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
	uris: HashSet<String>,
	size: u64,
	path: String,
	hash_type: String,
	filename: String,
	progress: Progress,
}

impl URI {
	async fn from_version<'a>(
		version: &'a Version<'a>,
		cache: &Cache,
		config: &Config,
		downloader: &mut Downloader,
	) -> Result<URI> {
		let progress = Progress::new(version.size());

		Ok(URI {
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
			progress,
		})
	}

	fn dummy() -> Self {
		URI {
			uris: HashSet::new(),
			size: 10241024,
			path: "".to_string(),
			hash_type: "".to_string(),
			filename: "dummy-data.deb".to_string(),
			progress: Progress::new(10241024),
		}
	}
}

pub struct Progress {
	indicatif: ProgressBar,
	bytes_per_sec: String,
	current_total: String,
	percentage: String,
}

impl Progress {
	fn new(total: u64) -> Self {
		let indicatif = ProgressBar::hidden();
		indicatif.set_length(total);

		Progress {
			indicatif,
			bytes_per_sec: String::new(),
			current_total: String::new(),
			percentage: String::new(),
		}
	}

	fn ratio(&self) -> f64 {
		self.indicatif.position() as f64 / self.indicatif.length().unwrap() as f64
	}

	fn update_strings(&mut self) {
		self.bytes_per_sec = format!(
			"{}/s",
			unit_str(self.indicatif.per_sec() as u64, NumSys::Binary)
		);
		self.current_total = format!(
			"{}/{}",
			unit_str(self.indicatif.position(), NumSys::Binary),
			unit_str(self.indicatif.length().unwrap(), NumSys::Binary)
		);
		self.percentage = format!("{:.1} %", self.ratio() * 100.0);
	}

	fn bytes_per_sec(&self) -> &str { &self.bytes_per_sec }

	fn current_total(&self) -> &str { &self.current_total }

	fn percentage(&self) -> &str { &self.percentage }

	fn bar_length(&self) -> u16 {
		(terminal_width()
			- (self.percentage().len()
				+ self.current_total().len()
				+ self.bytes_per_sec().len()
				+ 8)) as u16
	}
}

pub struct Downloader {
	uri_list: Vec<URI>,
	untrusted: HashSet<String>,
	not_found: Vec<String>,
	mirrors: HashMap<String, String>,
	mirror_regex: MirrorRegex,
	progress: Progress,
	total_pkgs: usize,
	finished: usize,
}

impl Downloader {
	fn new() -> Self {
		Downloader {
			uri_list: vec![],
			untrusted: HashSet::new(),
			not_found: vec![],
			mirrors: HashMap::new(),
			mirror_regex: MirrorRegex::new(),
			progress: Progress::new(0),
			total_pkgs: 0,
			finished: 0,
		}
	}

	fn set_total(&mut self) {
		self.total_pkgs = self.uri_list.len();
		let total = self
			.uri_list
			.iter()
			.map(|uri| uri.progress.indicatif.length().unwrap())
			.sum();
		self.progress.indicatif.set_length(total);
	}

	// fn ratio(&self) -> f64 {
	// 	self.progress.indicatif.position() as f64 /
	// self.progress.indicatif.length().unwrap() as f64 }

	// /// This will advance the other progress bar and the total at once.
	// fn advance_progress(&self, other_bar: &URI, length: u64) {
	// 	other_bar.progress.indicatif.inc(length);
	// 	self.progress.inc(length);
	// }
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

pub fn uri_trusted<'a>(cache: &Cache, version: &'a Version<'a>) -> Result<bool> {
	for mut pf in version.package_files() {
		if pf.archive()? != "now" {
			return Ok(cache.is_trusted(&mut pf));
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

		if !uri_trusted(cache, version)? {
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
						std::fs::read_to_string(filename).with_context(|| {
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

	// Must set the total from the URIs we just gathered.
	downloader.set_total();

	// setup terminal
	enable_raw_mode()?;
	let mut stdout = std::io::stdout();
	execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
	let backend = CrosstermBackend::new(stdout);
	// let mut terminal = Terminal::new(backend)?;
	let mut terminal = Terminal::with_options(
		backend,
		TerminalOptions {
			viewport: Viewport::Fullscreen,
		},
	)?;

	// create app and run it
	let tick_rate = Duration::from_millis(250);
	// let app = App::new();
	let res = run_app(&mut terminal, &mut downloader, tick_rate, config);

	// restore terminal
	disable_raw_mode()?;
	execute!(
		terminal.backend_mut(),
		LeaveAlternateScreen,
		DisableMouseCapture
	)?;
	terminal.show_cursor()?;

	if let Err(err) = res {
		println!("{err:?}");
	}

	// let mut set = JoinSet::new();
	// for uri in downloader.uri_list {
	// 	let pkg_name = config.color.package(&uri.filename).to_string();
	// 	set.spawn(download_file(progress.clone(), uri.clone(), pkg_name));
	// }

	// while let Some(res) = set.join_next().await {
	// 	let _out = res??;
	// }

	Ok(())
}

fn run_app<B: Backend>(
	terminal: &mut Terminal<B>,
	mut downloader: &mut Downloader,
	tick_rate: Duration,
	config: &Config,
) -> std::io::Result<()> {
	let mut last_tick = Instant::now();
	loop {
		terminal.draw(|f| ui(f, &mut downloader, config))?;

		for uri in downloader.uri_list.iter_mut() {
			if (uri.progress.indicatif.position() + 102400)
				>= uri.progress.indicatif.length().unwrap()
			{
				continue;
			}
			uri.progress.indicatif.inc(102400);
			downloader.progress.indicatif.inc(102400);


		}

		let timeout = tick_rate
			.checked_sub(last_tick.elapsed())
			.unwrap_or_else(|| Duration::from_secs(0));
		if crossterm::event::poll(timeout)? {
			if let Event::Key(key) = event::read()? {
				if let KeyCode::Char('q') = key.code {
					return Ok(());
				}
			}
		}
		if last_tick.elapsed() >= tick_rate {
			// app.on_tick();
			last_tick = Instant::now();
		}
	}
}

fn ui<B: Backend>(f: &mut Frame<B>, downloader: &mut Downloader, config: &Config) {
	let mut constraints = vec![];

	for _item in &downloader.uri_list {
		constraints.push(Constraint::Max(1))
	}

	constraints.push(Constraint::Min(5));
	// Last Constraint stops expansion
	constraints.push(Constraint::Min(0));

	let outer_block = Block::new()
		.borders(Borders::ALL)
		.border_type(BorderType::Rounded)
		.title("  Downloading...  ".reset().bold())
		.style(
			Style::default()
				.fg(Color::Cyan)
				.add_modifier(Modifier::BOLD),
		);

	let inner = outer_block.inner(f.size());
	f.render_widget(outer_block, f.size());

	let chunks = Layout::default()
		.direction(Direction::Vertical)
		.constraints(constraints)
		.split(inner);

	let mut total = 0;
	let mut bar_length = 1024;
	let mut current_total_length = 0;
	let mut bytes_per_second_length = 0;
	let mut percentage_length = 0;
	// This section is just for aligning columns
	for uri in downloader.uri_list.iter_mut() {
		uri.progress.update_strings();
		if uri.filename.len() > total {
			total = uri.filename.len()
		}

		if uri.progress.bar_length() < bar_length {
			bar_length = uri.progress.bar_length();
		}

		if uri.progress.current_total.len() > current_total_length {
			current_total_length = uri.progress.current_total.len();
		}

		if uri.progress.percentage.len() > percentage_length {
			percentage_length = uri.progress.percentage.len();
		}

		if uri.progress.bytes_per_sec.len() > bytes_per_second_length {
			bytes_per_second_length = uri.progress.bytes_per_sec.len();
		}
	}

	// This portion is what actually renders the progress bars.
	// You can see we use some of that data from above
	// and pad the strings if necessary.
	for (i, uri) in downloader.uri_list.iter().enumerate() {
		let first_column = match uri.filename.len() < total {
			true => uri.filename.to_string() + &" ".repeat(total - uri.filename.len()),
			false => uri.filename.to_string(),
		};

		let gauge = progress_widget(first_column.reset().bold(), &uri.progress);

		let new_chunk = split_horizontal(
			[
				// We calculated how big each needed to be
				// So that they are aligned properly.
				Constraint::Length(bar_length),
				Constraint::Length(percentage_length as u16 + 2),
				Constraint::Length(current_total_length as u16 + 2),
				Constraint::Length(bytes_per_second_length as u16 + 2),
			],
			chunks[i],
		);

		f.render_widget(gauge, new_chunk[0]);
		f.render_widget(get_paragraph(uri.progress.percentage()), new_chunk[1]);
		f.render_widget(get_paragraph(uri.progress.current_total()), new_chunk[2]);
		f.render_widget(get_paragraph(uri.progress.bytes_per_sec()), new_chunk[3]);
	}

	// Padding buffer
	// Adds a blank space between individual downloads and the total section
	// f.render_widget(Block::new(), chunks[downloader.uri_list.len()]);

	// This is where we build the "block" to render things inside.
	let total_block = Block::default()
		.borders(Borders::ALL)
		.title("  Total Progress...  ".reset().bold())
		.border_type(BorderType::Rounded)
		.title_alignment(Alignment::Center);

	// We now create the inner block for the total block
	let total_inner = total_block.inner(chunks[downloader.uri_list.len()]);
	let total_inner_block = Layout::default()
		.direction(Direction::Vertical)
		.constraints([Constraint::Min(1), Constraint::Min(1)])
		.split(total_inner);
	f.render_widget(total_block, chunks[downloader.uri_list.len()]);

	// Update our information
	downloader.progress.update_strings();

	f.render_widget(
		Paragraph::new("Packages: 0/12")
		.wrap(Wrap { trim: true })
		.alignment(Alignment::Left)
		.set_style(Style::default().fg(Color::White)),
		total_inner_block[0],
	);

	// We split the row horizontally so that we can organize information
	let total_chunk = split_horizontal(
		[
			Constraint::Length((downloader.progress.bar_length() - 2) as u16),
			Constraint::Length(downloader.progress.percentage().len() as u16 + 2),
			Constraint::Length(downloader.progress.current_total().len() as u16 + 2),
			Constraint::Length(downloader.progress.bytes_per_sec().len() as u16 + 2),
		],
		total_inner_block[1],
	);

	// Get the progress widget
	let total_progress = progress_widget(
		format!(
			"Time Remaining: {}",
			FormattedDuration(downloader.progress.indicatif.eta())
		),
		&downloader.progress,
	);

	// Finally render all of the bars for this row
	f.render_widget(total_progress, total_chunk[0]);
	f.render_widget(
		get_paragraph(downloader.progress.percentage()),
		total_chunk[1],
	);
	f.render_widget(
		get_paragraph(downloader.progress.current_total()),
		total_chunk[2],
	);
	f.render_widget(
		get_paragraph(downloader.progress.bytes_per_sec()),
		total_chunk[3],
	);
}

// Splits a block horizontally with your contraints
fn split_horizontal<T: Into<Vec<Constraint>>>(
	constraints: T,
	block: Rect,
) -> Rc<[Rect]> {
	Layout::default()
		.direction(Direction::Horizontal)
		.constraints(constraints)
		.split(block)
}

fn progress_widget<'a, T: Into<Line<'a>>>(label: T, progress: &'a Progress) -> LineGauge<'a> {
	LineGauge::default()
		.line_set(symbols::line::THICK)
		.ratio(progress.ratio())
		.label(label).style(Style::default().fg(Color::White))
		.gauge_style(Style::default().fg(Color::Cyan).bg(Color::Red))
}

fn get_paragraph(text: &str) -> Paragraph {
	Paragraph::new(text)
		.wrap(Wrap { trim: true })
		.alignment(Alignment::Right)
		.set_style(Style::default().fg(Color::White))
}

pub async fn download_file(
	progress: Arc<Mutex<Progress>>,
	uri: Arc<URI>,
	pkg_name: String,
) -> Result<()> {
	let client = reqwest::Client::new();
	let mut response = client.get(uri.uris.iter().next().unwrap()).send().await?;

	// let pb = progress
	// 	.lock()
	// 	.unwrap()
	// 	.multi
	// 	.insert_from_back(1, ProgressBar::new(uri.size));
	// pb.set_style(
	// 	ProgressStyle::with_template(
	// 		"│  {msg} [{wide_bar:.cyan/red}] {percent}% • {bytes}/{total_bytes} • \
	// 		 {binary_bytes_per_sec}  │",
	// 	)
	// 	.unwrap()
	// 	.progress_chars("━━━"),
	// );

	// pb.set_message(pkg_name);

	// while let Some(chunk) = response.chunk().await? {
	// 	progress.lock().unwrap().progress.inc(chunk.len() as u64);
	// 	pb.inc(chunk.len() as u64);
	// }
	// pb.finish();

	// let mut total_pb = progress.lock().unwrap();
	// total_pb.pkgs_downloaded += 1;
	// total_pb.progress.set_message(format!(
	// 	"{}/{}",
	// 	total_pb.pkgs_downloaded, total_pb.total_pkgs
	// ));

	Ok(())
}