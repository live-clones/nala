use std::io::Stdout;
use std::rc::Rc;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{Event, KeyCode};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use crossterm::{cursor, event, execute, ExecutableCommand};
use indicatif::{ProgressBar, ProgressStyle};
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::layout::{Alignment, Constraint, Rect};
use ratatui::style::{Color, Modifier, Style, Styled, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{LineGauge, Paragraph, Widget, Wrap};
use ratatui::{symbols, Frame, Terminal, TerminalOptions, Viewport};
use rust_apt::error::{pending_error, AptErrors};
use rust_apt::progress::{AcquireProgress, DynAcquireProgress};
use rust_apt::raw::{AcqTextStatus, ItemDesc, ItemState, PkgAcquire};
use rust_apt::util::{terminal_width, time_str, unit_str, NumSys};
use rust_apt::{new_cache, Cache, PackageSort};
use tokio::task::JoinSet;

use crate::config::Config;

struct NalaProgressBar {
	indicatif: ProgressBar,
	header: String,
}

impl NalaProgressBar {
	fn new(header: String) -> Self {
		Self {
			indicatif: ProgressBar::hidden(),
			header,
		}
	}

	fn ratio(&self) -> f64 {
		self.indicatif.position() as f64 / self.indicatif.length().unwrap() as f64
	}

	fn render(&mut self, f: &mut Frame, msg: Vec<Span>) {
		let block = crate::downloader::build_block(self.header.to_string().reset().bold());
		let inner = crate::downloader::split_vertical(
			[Constraint::Length(1), Constraint::Length(1)],
			block.inner(f.size()),
		);
		f.render_widget(block, f.size());

		f.render_widget(Paragraph::new(Line::from(msg)), inner[0]);

		let bar_block = crate::downloader::split_horizontal(
			[
				Constraint::Fill(100),
				Constraint::Fill(5),
				Constraint::Fill(5),
				Constraint::Fill(5),
				Constraint::Min(0),
			],
			inner[1],
		);

		f.render_widget(
			LineGauge::default()
				.line_set(symbols::line::THICK)
				.ratio(self.ratio())
				.label("Downloading...")
				.style(Style::default().fg(Color::White))
				.gauge_style(Style::default().fg(Color::LightGreen).bg(Color::Red)),
			bar_block[0],
		);
		f.render_widget(
			crate::downloader::get_paragraph(&format!("{:.1} %", self.ratio() * 100.0)),
			bar_block[1],
		);
		f.render_widget(
			crate::downloader::get_paragraph(&format!(
				"{}/{}",
				unit_str(self.indicatif.position(), NumSys::Binary),
				unit_str(self.indicatif.length().unwrap(), NumSys::Binary)
			)),
			bar_block[2],
		);
		f.render_widget(
			crate::downloader::get_paragraph(&format!(
				"{}/s",
				unit_str(self.indicatif.per_sec() as u64, NumSys::Binary)
			)),
			bar_block[3],
		);
	}
}

fn should_quit() -> Result<bool> {
	if event::poll(Duration::from_millis(250)).context("event poll failed")? {
		if let Event::Key(key) = event::read().context("event read failed")? {
			return Ok(KeyCode::Char('q') == key.code);
		}
	}
	Ok(false)
}

pub fn update(config: &Config) -> Result<(), AptErrors> {
	let cache = new_cache!()?;
	enable_raw_mode()?;
	let mut terminal = Terminal::with_options(
		CrosstermBackend::new(std::io::stdout()),
		TerminalOptions {
			viewport: Viewport::Inline(4),
		},
	)?;

	// TODO: Handle CtrlC here so that we can unhide the cursor

	// I think NalaAcquireProgress will have to store terminal
	// And then we will need a defined ui function, maybe self.ui? It depends.
	// It doesn't need to be async I think
	// But it needs to be able to draw the terminal from the callbacks.
	// This might be weird, but idk that we can give everything send that needs it.
	//
	// terminal.draw(|f| ui(f, align, bars, constraints, pkgs_finished,
	// downloader))?;

	// run(&mut terminal)?;

	// disable_raw_mode()?;
	// terminal.clear()?;

	let res = cache.update(&mut AcquireProgress::new(NalaAcquireProgress::new(
		config, terminal,
	)));

	disable_raw_mode()?;
	terminal.clear()?;
	// Do not print how many packages are upgradable if update errored.
	#[allow(clippy::question_mark)]
	if res.is_err() {
		return res;
	}

	// let cache = new_cache!()?;
	// let sort = PackageSort::default().upgradable();
	// let upgradable: Vec<_> = cache.packages(&sort).collect();

	// if !upgradable.is_empty() {
	// 	println!(
	// 		"{} packages can be upgraded. Run '{}' to see them.",
	// 		config.color.yellow(&format!("{}", upgradable.len())),
	// 		config.color.package("nala list --upgradable")
	// 	);
	// }

	// Not sure yet if I want to implement this directly
	// But here is how one might do it.
	//
	// for pkg in upgradable {
	// 	let (Some(inst), Some(cand)) = (pkg.installed(), pkg.candidate()) else {
	// 		continue;
	// 	};

	// 	println!("{pkg} ({inst}) -> ({cand})");
	// }

	res
}

// "┏━┳┓\n"
// "┃ ┃┃\n"
// "┣━╋┫\n"
// "┃ ┃┃\n"
// "┣━╋┫\n"
// "┣━╋┫\n"
// "┃ ┃┃\n"
// "┗━┻┛\n"

// "╭─┬╮\n"
// "│ ││\n"
// "├─┼┤\n"
// "│ ││\n"
// "├─┼┤\n"
// "├─┼┤\n"
// "│ ││\n"
// "╰─┴╯\n"

const HEAVY: [&str; 6] = ["━", "┃", "┏", "┓", "┗", "┛"];
#[allow(dead_code)]
const ROUNDED: [&str; 6] = ["─", "│", "╭", "╮", "╰", "╯"];
#[allow(dead_code)]
const ASCII: [&str; 6] = ["-", "|", "x", "x", "x", "x"];

pub struct Border {
	chars: [&'static str; 6],
	width: usize,
	top: String,
	bot: String,
}

impl Border {
	pub fn new(chars: [&'static str; 6]) -> Border {
		Self {
			chars,
			width: 0,
			top: String::new(),
			bot: String::new(),
		}
	}

	pub fn horizontal(&self) -> &str { self.chars[0] }

	pub fn vertical(&self) -> &str { self.chars[1] }

	pub fn width_changed(&mut self) -> bool {
		let width = terminal_width();
		if self.width != width {
			self.width = width;
			return true;
		}
		false
	}

	pub fn top_border(&mut self) -> &str {
		if self.width_changed() || self.top.is_empty() {
			let header = "Update";

			self.top = format!(
				"{}{} {header} {}{}",
				self.chars[2],
				self.horizontal(),
				self.horizontal().repeat(self.width - header.len() - 5),
				self.chars[3],
			);
		}
		&self.top
	}

	pub fn bottom_border(&mut self) -> &str {
		if self.width_changed() || self.bot.is_empty() {
			self.bot = format!(
				"{}{}{}",
				self.chars[4],
				self.horizontal().repeat(self.width - 2),
				self.chars[5],
			)
		}
		&self.bot
	}
}

/// AptAcquireProgress is the default struct for the update method on the cache.
///
/// This struct mimics the output of `apt update`.
pub struct NalaAcquireProgress<'a, B: Backend> {
	config: &'a Config,
	terminal: Terminal<B>,
	pulse_interval: usize,
	max: usize,
	progress: NalaProgressBar,
	border: Border,
	ign: String,
	hit: String,
	get: String,
	err: String,
}

impl<'a, B: Backend> NalaAcquireProgress<'a, B> {
	/// Returns a new default progress instance.
	pub fn new(config: &'a Config, terminal: Terminal<B>) -> Self {
		let progress = Self {
			config,
			terminal,
			pulse_interval: 0,
			max: 0,
			progress: NalaProgressBar::new("  Update  ".to_string()),
			// TODO: Maybe we should make it configurable.
			border: Border::new(HEAVY),
			ign: config.color.yellow("Ignored").into(),
			hit: config.color.package("No Change").into(),
			get: config.color.blue("Updated").into(),
			err: config.color.red("Error").into(),
		};
		// progress.progress = ProgressBar::new(0)
		// 	.with_style(progress.get_style("".to_string()))
		// 	.with_prefix("%");

		// progress.progress.enable_steady_tick(Duration::from_secs(1));
		progress
	}

	pub fn print(&mut self, msg: &str) {
		self.terminal
			.insert_before(1, |buf| {
				Paragraph::new(msg)
					.wrap(Wrap { trim: true })
					.alignment(Alignment::Left)
					.set_style(Style::default().fg(Color::White))
					.render(buf.area, buf);
			})
			.unwrap();
	}

	pub fn get_style(&mut self, msg: String) -> ProgressStyle {
		// "┏━┳┓\n"
		// "┃ ┃┃\n"
		// "┣━╋┫\n"
		// "┃ ┃┃\n"
		// "┣━╋┫\n"
		// "┣━╋┫\n"
		// "┃ ┃┃\n"
		// "┗━┻┛\n"

		// "╭─┬╮\n"
		// "│ ││\n"
		// "├─┼┤\n"
		// "│ ││\n"
		// "├─┼┤\n"
		// "├─┼┤\n"
		// "│ ││\n"
		// "╰─┴╯\n"

		// TODO: This will need to be methods on the struct
		// And we will have to shift it around to be ASCII safe maybe.
		let mut template = self
			.config
			.color
			.package(self.border.top_border())
			.to_string();

		template += "\n";
		if !msg.is_empty() {
			template += &msg;
			template += "\n";
		}

		let side = self
			.config
			.color
			.package(self.border.vertical())
			.to_string();

		template += &format!(
			"{side} {{spinner:.bold}} {{percent:.bold}}{{prefix:.bold}} [{{wide_bar:.cyan/red}}] \
			 {{bytes}}/{{total_bytes}} ({{eta}}) {side}\n{}",
			self.config.color.package(self.border.bottom_border())
		);

		ProgressStyle::with_template(&template)
			.unwrap()
			.tick_strings(&[".  ", ".. ", "...", "   "])
			.progress_chars("━━")
	}
}

impl<'a, B: Backend> DynAcquireProgress for NalaAcquireProgress<'a, B> {
	/// Used to send the pulse interval to the apt progress class.
	///
	/// Pulse Interval is in microseconds.
	///
	/// Example: 1 second = 1000000 microseconds.
	///
	/// Apt default is 500000 microseconds or 0.5 seconds.
	///
	/// The higher the number, the less frequent pulse updates will be.
	///
	/// Pulse Interval set to 0 assumes the apt defaults.
	fn pulse_interval(&self) -> usize { self.pulse_interval }

	/// Called when an item is confirmed to be up-to-date.
	///
	/// Prints out the short description and the expected size.
	fn hit(&mut self, item: &ItemDesc) {
		self.print(&format!("{}: {}", self.hit, item.description()))
		// self.progress
		// 	.println(format!("{}: {}", self.hit, item.description()));
	}

	/// Called when an Item has started to download
	///
	/// Prints out the short description and the expected size.
	fn fetch(&mut self, item: &ItemDesc) {
		// let mut msg = format!("{}:   {}", self.get, item.description());

		// let file_size = item.owner().file_size();
		// if file_size != 0 {
		// 	msg += &format!(" [{}]", unit_str(file_size, NumSys::Decimal))
		// }

		// self.progress.println(msg);
	}

	/// Called when an item is successfully and completely fetched.
	///
	/// We don't print anything here to remain consistent with apt.
	fn done(&mut self, _: &ItemDesc) {}

	/// Called when progress has started.
	///
	/// Start does not pass information into the method.
	///
	/// We do not print anything here to remain consistent with apt.
	fn start(&mut self) {}

	/// Called when progress has finished.
	///
	/// Stop does not pass information into the method.
	///
	/// prints out the bytes downloaded and the overall average line speed.
	fn stop(&mut self, status: &AcqTextStatus) {
		// if pending_error() {
		// 	return;
		// }

		// let msg = if status.fetched_bytes() != 0 {
		// 	self.config
		// 		.color
		// 		.bold(&format!(
		// 			"Fetched {} in {} ({}/s)",
		// 			unit_str(status.fetched_bytes(), NumSys::Decimal),
		// 			time_str(status.elapsed_time()),
		// 			unit_str(status.current_cps(), NumSys::Decimal)
		// 		))
		// 		.to_string()
		// } else {
		// 	"Nothing to fetch.".to_string()
		// };
		// self.progress.println(msg);
	}

	/// Called when an Item fails to download.
	///
	/// Print out the ErrorText for the Item.
	fn fail(&mut self, item: &ItemDesc) {
		// let mut show_error = self
		// 	.config
		// 	.apt
		// 	.bool("Acquire::Progress::Ignore::ShowErrorText", true);
		// let error_text = item.owner().error_text();

		// let header = match item.owner().status() {
		// 	ItemState::StatIdle | ItemState::StatDone => {
		// 		if error_text.is_empty() {
		// 			show_error = false;
		// 		}
		// 		&self.ign
		// 	},
		// 	_ => &self.err,
		// };

		// self.progress
		// 	.println(format!("{header}: {}", item.description()));

		// if show_error {
		// 	self.progress.println(error_text);
		// }
	}

	/// Called periodically to provide the overall progress information
	///
	/// Draws the current progress.
	/// Each line has an overall percent meter and a per active item status
	/// meter along with an overall bandwidth and ETA indicator.
	fn pulse(&mut self, status: &AcqTextStatus, owner: &PkgAcquire) {
		self.progress.indicatif.set_length(status.total_bytes());
		self.progress.indicatif.set_position(status.current_bytes());

		if should_quit().unwrap() {
			panic!("Find a better way to exit!")
		}

		let mut string: Vec<Span> = vec![];

		for worker in owner.workers().iter() {
			let Ok(item) = worker.item() else {
				continue;
			};

			let owner = item.owner();
			if owner.status() != ItemState::StatFetching {
				continue;
			}

			let mut work_string = owner.active_subprocess();
			if work_string.is_empty() {
				work_string += "Downloading"
			} else if work_string == "store" {
				work_string = "Processing".to_string()
			}
			work_string += ": ";

			if let Some(dest_file) = owner.dest_file().split_terminator('/').last() {
				// Decide on protocol.
				let proto = if item.uri().starts_with("https") { "https://" } else { "http://" };
				// Build the correct URI by destination file.
				// item.uri() returns the /by-hash link.
				let mut uri = dest_file.replace('_', "/");
				uri.insert_str(0, proto);

				string.push(Span::from(work_string).white().bold());

				string.push(Span::from(uri).reset().white());

				// Break only for slim progress
				if !self.config.get_bool("verbose", false) {
					break;
				}
			};
		}

		if string.is_empty() {
			string.push(Span::from("Working...").white().bold())
		}
		self.terminal
			.draw(|f| self.progress.render(f, string))
			.unwrap();

		// self.max = string.len().max(self.max);

		// let filler = format!("{side}{}{side}", &" ".repeat(term_width - 2));

		// while string.len() < self.max {
		// 	string.insert(0, filler.to_string())
		// }

		// let style = self.get_style(string.join("\n"));
		// self.progress.set_style(style);
	}
}
