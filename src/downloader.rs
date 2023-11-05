use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{bail, Context, Result};
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode};
use crossterm::execute;
use crossterm::terminal::{
	disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use indicatif::{FormattedDuration, ProgressBar};
use once_cell::sync::OnceCell;
use ratatui::prelude::*;
use ratatui::style::Stylize;
use ratatui::widgets::block::Title;
use ratatui::widgets::*;
use regex::{Regex, RegexBuilder};
use rust_apt::cache::Cache;
use rust_apt::new_cache;
use rust_apt::package::Version;
use rust_apt::records::RecordField;
use rust_apt::util::{terminal_width, unit_str, NumSys};
use tokio::sync::{Mutex, MutexGuard};
use tokio::task::JoinSet;
use tokio::time::Duration;

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
pub struct Uri {
	uris: HashSet<String>,
	size: u64,
	path: String,
	hash_type: String,
	filename: String,
	progress: Progress,
}

impl Uri {
	async fn from_version<'a>(
		version: &'a Version<'a>,
		cache: &Cache,
		config: &Config,
		downloader: &mut Downloader,
	) -> Result<Arc<Mutex<Uri>>> {
		let progress = Progress::new(version.size());

		Ok(Arc::new(Mutex::new(Uri {
			uris: downloader.filter_uris(version, cache, config).await?,
			size: version.size(),
			path: "".to_string(),
			hash_type: "".to_string(),
			filename: version
				.get_record(RecordField::Filename)
				.expect("Record does not contain a filename!")
				.split_terminator('/')
				.last()
				.expect("Filename is malformed!")
				.to_string(),
			progress,
		})))
	}
}

/// Struct used for sending progress data to UI
struct SubBar {
	is_total: bool,
	first_column: String,
	percentage: String,
	current_total: String,
	bytes_per_sec: String,
	ratio: f64,
}

impl SubBar {
	/// Consume the Bar and render it to the terminal.
	fn render<B: Backend>(self, f: &mut Frame<B>, chunk: Rc<[Rect]>) {
		f.render_widget(
			LineGauge::default()
				.line_set(symbols::line::THICK)
				.ratio(self.ratio)
				.label(self.first_column)
				.style(Style::default().fg(Color::White))
				.gauge_style(Style::default().fg(Color::Cyan).bg(Color::Red)),
			chunk[0],
		);
		f.render_widget(get_paragraph(&self.percentage), chunk[1]);
		f.render_widget(get_paragraph(&self.current_total), chunk[2]);
		f.render_widget(get_paragraph(&self.bytes_per_sec), chunk[3]);
	}
}

/// Struct used for aligning the progress segments
struct BarAlignment {
	pkg_name: usize,
	bar: u16,
	len: usize,
	current_total: usize,
	bytes_per_sec: usize,
	percentage: usize,
}

impl BarAlignment {
	fn new() -> Self {
		BarAlignment {
			pkg_name: 0,
			bar: 1024,
			len: 0,
			current_total: 0,
			bytes_per_sec: 0,
			percentage: 0,
		}
	}

	fn update_from_uri(&mut self, uri: MutexGuard<Uri>) {
		if uri.filename.len() > self.pkg_name {
			self.pkg_name = uri.filename.len()
		}

		if uri.progress.bar_length() < self.bar {
			self.bar = uri.progress.bar_length();
		}

		if uri.progress.current_total.len() > self.current_total {
			self.current_total = uri.progress.current_total.len();
		}

		if uri.progress.percentage.len() > self.percentage {
			self.percentage = uri.progress.percentage.len();
		}

		if uri.progress.bytes_per_sec.len() > self.bytes_per_sec {
			self.bytes_per_sec = uri.progress.bytes_per_sec.len();
		}

		// Increase the amount of downloads
		self.len += 1;
	}

	fn constraints(&self) -> [Constraint; 4] {
		[
			Constraint::Length(self.bar),
			Constraint::Length(self.percentage as u16 + 2),
			Constraint::Length(self.current_total as u16 + 2),
			Constraint::Length(self.bytes_per_sec as u16 + 2),
		]
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
	uri_list: Vec<Arc<Mutex<Uri>>>,
	untrusted: HashSet<String>,
	not_found: Vec<String>,
	mirrors: HashMap<String, String>,
	mirror_regex: MirrorRegex,
	progress: Arc<Mutex<Progress>>,
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
			progress: Arc::new(Mutex::new(Progress::new(0))),
			total_pkgs: 0,
			finished: 0,
		}
	}

	/// Set the total for total progress based on the totals for Uri Progress.
	async fn set_total(&mut self) {
		self.total_pkgs = self.uri_list.len();
		let mut total = 0;
		for uri in &self.uri_list {
			total += uri.lock().await.progress.indicatif.length().unwrap();
		}
		self.progress.lock().await.indicatif.set_length(total);
	}

	/// Add the filtered Uris into the HashSet if applicable.
	fn get_from_mirrors<'a>(
		&self,
		version: &'a Version<'a>,
		uris: &mut HashSet<String>,
		filename: &str,
	) -> Result<bool> {
		if let Some(data) = self.mirrors.get(filename) {
			for line in data.lines() {
				if !line.is_empty() && !line.starts_with('#') {
					uris.insert(
						line.to_string()
							+ "/" + &version.get_record(RecordField::Filename).unwrap(),
					);
				}
			}
			return Ok(true);
		}
		Ok(false)
	}

	async fn add_to_mirrors(&mut self, uri: &str, filename: &str) -> Result<()> {
		self.mirrors.insert(
			filename.to_string(),
			match uri.starts_with("mirror+file:") {
				true => std::fs::read_to_string(filename)
					.with_context(|| format!("Failed to read {filename}, using defaults"))?,
				false => {
					reqwest::get("http://".to_string() + filename)
						.await?
						.text()
						.await?
				},
			},
		);
		Ok(())
	}

	/// Filter Uris from a package version.
	/// This will normalize different kinds of possible Uris
	/// Which are not http
	async fn filter_uris<'a>(
		&mut self,
		version: &'a Version<'a>,
		cache: &Cache,
		config: &Config,
	) -> Result<HashSet<String>> {
		let mut filtered = HashSet::new();

		for uri in version.uris() {
			// Sending a file path through the downloader will cause it to lock up
			// These have already been handled before the downloader runs.
			// TODO: We haven't actually handled anything yet. In python nala it happens
			// before it gets here. lol
			if uri.starts_with("file:") {
				continue;
			}

			if !uri_trusted(cache, version)? {
				self.untrusted
					.insert(config.color.red(version.parent().name()).to_string());
			}

			if uri.starts_with("mirror+file:") || uri.starts_with("mirror:") {
				if let Some(file_match) = self.mirror_regex.mirror_file()?.captures(&uri) {
					let filename = file_match.get(1).unwrap().as_str();
					if !self.mirrors.contains_key(filename) {
						self.add_to_mirrors(&uri, filename).await?;
					};

					if self.get_from_mirrors(version, &mut filtered, filename)? {
						continue;
					}
				}
			}

			// If none of the conditions meet then we just add it to the uris
			filtered.insert(uri);
		}
		Ok(filtered)
	}
}

pub fn uri_trusted<'a>(cache: &Cache, version: &'a Version<'a>) -> Result<bool> {
	for mut pf in version.package_files() {
		// TODO: There is a bug here with codium specifically
		// It doesn't have an archive. Check apt source for clues here.
		if pf.archive()? != "now" {
			return Ok(cache.is_trusted(&mut pf));
		}
	}
	Ok(false)
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
							Uri::from_version(version, &cache, config, &mut downloader).await?;
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

	// Must set the total from the Uris we just gathered.
	downloader.set_total().await;

	// Set up the futures
	let mut set = JoinSet::new();
	for uri in &downloader.uri_list {
		set.spawn(download_file(downloader.progress.clone(), uri.clone()));
	}

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
	let res = run_app(&mut terminal, &mut downloader, tick_rate).await;

	// This is for closing out of the app.
	if res.is_ok() {
		disable_raw(&mut terminal)?;
		set.abort_all();
	}

	// Run all of the futures.
	while let Some(res) = set.join_next().await {
		res??;
	}

	disable_raw(&mut terminal)?;

	if let Err(err) = res {
		println!("{err:?}");
	}

	Ok(())
}

async fn run_app<B: Backend>(
	terminal: &mut Terminal<B>,
	downloader: &mut Downloader,
	tick_rate: Duration,
) -> std::io::Result<()> {
	let mut last_tick = Instant::now();

	loop {
		// Calculate the alignment for rendering.
		let mut align = BarAlignment::new();
		for uri in downloader.uri_list.iter_mut() {
			let mut unlocked = uri.lock().await;
			unlocked.progress.update_strings();

			align.update_from_uri(unlocked);
		}

		// Build the progress bar data for individual packages.
		let mut sub_bars = vec![];
		for uri in &downloader.uri_list {
			let unlocked = uri.lock().await;

			sub_bars.push(SubBar {
				is_total: false,
				// Pad the package name if necessary for alignment.
				first_column: unlocked.filename.to_string()
					+ &" ".repeat(align.pkg_name - unlocked.filename.len()),
				ratio: unlocked.progress.ratio(),
				percentage: unlocked.progress.percentage().to_string(),
				current_total: unlocked.progress.current_total().to_string(),
				bytes_per_sec: unlocked.progress.bytes_per_sec().to_string(),
			})
		}

		// Update our total information
		let mut unlocked = downloader.progress.lock().await;
		unlocked.update_strings();

		// Total constraints have to be built outside of UI
		let total_constraints = vec![
			Constraint::Length((unlocked.bar_length() - 2) as u16),
			Constraint::Length(unlocked.percentage().len() as u16 + 2),
			Constraint::Length(unlocked.current_total().len() as u16 + 2),
			Constraint::Length(unlocked.bytes_per_sec().len() as u16 + 2),
		];

		// Create a bar for the total progress.
		// It should always be last in the Vec.
		// TODO: Maybe SubBar isn't the best name?
		sub_bars.push(SubBar {
			is_total: true,
			first_column: format!(
				"Time Remaining: {}",
				FormattedDuration(unlocked.indicatif.eta())
			),
			percentage: unlocked.percentage().to_string(),
			current_total: unlocked.current_total().to_string(),
			bytes_per_sec: unlocked.bytes_per_sec().to_string(),
			ratio: unlocked.ratio(),
		});

		terminal.draw(|f| ui(f, align, sub_bars, total_constraints))?;

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
			// TODO: We could potentially only update info at the tick rate.
			last_tick = Instant::now();
		}
	}
}

fn ui<B: Backend>(
	f: &mut Frame<B>,
	align: BarAlignment,
	sub_bars: Vec<SubBar>,
	total_constraints: Vec<Constraint>,
) {
	// Create the outer downloading block
	let outer_block = build_block("  Downloading...  ".reset().bold());
	let chunks = split_vertical(uri_constraints(align.len), outer_block.inner(f.size()));

	// This is where we build the "block" to render things inside.
	let total_block = build_block("  Total Progress...  ".reset().bold());

	// We now create the inner block for the total block
	let total_inner_block = split_vertical(
		[Constraint::Min(1), Constraint::Min(1)],
		total_block.inner(chunks[align.len]),
	);

	// Start rendering our blocks
	f.render_widget(outer_block, f.size());
	f.render_widget(total_block, chunks[align.len]);

	// Render the bars
	f.render_widget(
		Paragraph::new("Packages: 0/12")
			.wrap(Wrap { trim: true })
			.alignment(Alignment::Left)
			.set_style(Style::default().fg(Color::White)),
		total_inner_block[0],
	);

	// This portion renders the progress bars.
	// The total progress bar is last.
	for (i, bar) in sub_bars.into_iter().enumerate() {
		let new_chunk = match bar.is_total {
			true => split_horizontal(total_constraints.as_slice(), total_inner_block[1]),
			false => split_horizontal(align.constraints(), chunks[i]),
		};
		bar.render(f, new_chunk);
	}
}

/// Splits a block horizontally with your contraints
fn split_horizontal<T: Into<Vec<Constraint>>>(constraints: T, block: Rect) -> Rc<[Rect]> {
	Layout::default()
		.direction(Direction::Horizontal)
		.constraints(constraints)
		.split(block)
}

/// Splits a block vertically with your contraints
fn split_vertical<T: Into<Vec<Constraint>>>(constraints: T, block: Rect) -> Rc<[Rect]> {
	Layout::default()
		.direction(Direction::Vertical)
		.constraints(constraints)
		.split(block)
}

/// Build constraints based on how many downloads
fn uri_constraints(num: usize) -> Vec<Constraint> {
	let mut constraints = vec![
		// Last Constraint stops element expansion
		Constraint::Min(0),
	];

	for _ in 0..num {
		constraints.insert(0, Constraint::Max(1));
	}
	constraints
}

fn get_paragraph(text: &str) -> Paragraph {
	Paragraph::new(text)
		.wrap(Wrap { trim: true })
		.alignment(Alignment::Right)
		.set_style(Style::default().fg(Color::White))
}

fn build_block<'a, T: Into<Title<'a>>>(title: T) -> Block<'a> {
	Block::new()
		.borders(Borders::ALL)
		.border_type(BorderType::Rounded)
		.title_alignment(Alignment::Center)
		.title(title)
		.style(
			Style::default()
				.fg(Color::Cyan)
				.add_modifier(Modifier::BOLD),
		)
}

/// Restore Terminal
fn disable_raw<B: std::io::Write + Backend>(terminal: &mut Terminal<B>) -> Result<()> {
	disable_raw_mode()?;
	execute!(
		terminal.backend_mut(),
		LeaveAlternateScreen,
		DisableMouseCapture
	)?;
	terminal.show_cursor()?;
	Ok(())
}

pub async fn download_file(progress: Arc<Mutex<Progress>>, uri: Arc<Mutex<Uri>>) -> Result<()> {
	let client = reqwest::Client::new();
	let mut response = client
		.get(uri.lock().await.uris.iter().next().unwrap())
		.send()
		.await?;

	while let Some(chunk) = response.chunk().await? {
		progress.lock().await.indicatif.inc(chunk.len() as u64);
		uri.lock().await.progress.indicatif.inc(chunk.len() as u64);
	}

	Ok(())
}
