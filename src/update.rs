use std::time::Duration;

use anyhow::Result;
use crossterm::event;
use crossterm::event::{Event, KeyCode};
use ratatui::backend::Backend;
use ratatui::layout::Alignment;
use ratatui::style::{Color, Style, Styled, Stylize};
use ratatui::text::Span;
use ratatui::widgets::{Paragraph, Widget, Wrap};
use ratatui::Terminal;
use rust_apt::error::pending_error;
use rust_apt::progress::{AcquireProgress, DynAcquireProgress};
use rust_apt::raw::{AcqTextStatus, ItemDesc, ItemState, PkgAcquire};
use rust_apt::util::time_str;
use rust_apt::{new_cache, PackageSort};
use tokio::task::JoinHandle;

use crate::config::Config;
use crate::tui::progress::NalaProgressBar;
use crate::util::{init_terminal, restore_terminal};

pub fn poll_exit_event_loop() -> Result<()> {
	loop {
		if crossterm::event::poll(Duration::from_millis(250))? {
			if let Event::Key(key) = event::read()? {
				if let KeyCode::Char('q') = key.code {
					return Ok(());
				}
			}
		}
	}
}

pub fn poll_exit_event() -> Result<bool> {
	if crossterm::event::poll(Duration::from_millis(250))? {
		if let Event::Key(key) = event::read()? {
			if let KeyCode::Char('q') = key.code {
				return Ok(true);
			}
		}
	}
	Ok(false)
}

#[tokio::main]
pub async fn update(config: &Config) -> Result<()> {
	let cache = new_cache!()?;

	let mut terminal = init_terminal(true)?;

	let poll = tokio::task::spawn_blocking(poll_exit_event_loop);

	let res = cache.update(&mut AcquireProgress::new(NalaAcquireProgress::new(
		config,
		&mut terminal,
		&poll,
	)));

	// // Do not print how many packages are upgradable if update errored.
	#[allow(clippy::question_mark)]
	if res.is_err() {
		return Ok(res?);
	}

	let cache = new_cache!()?;
	let sort = PackageSort::default().upgradable();
	let upgradable: Vec<_> = cache.packages(&sort).collect();

	if !upgradable.is_empty() {
		println!(
			"{} packages can be upgraded. Run '{}' to see them.",
			config.color.yellow(&format!("{}", upgradable.len())),
			config.color.package("nala list --upgradable")
		);
	}

	// Not sure yet if I want to implement this directly
	// But here is how one might do it.
	//
	// for pkg in upgradable {
	// 	let (Some(inst), Some(cand)) = (pkg.installed(), pkg.candidate()) else {
	// 		continue;
	// 	};

	// 	println!("{pkg} ({inst}) -> ({cand})");
	// }

	Ok(res?)
}

/// AptAcquireProgress is the default struct for the update method on the cache.
///
/// This struct mimics the output of `apt update`.
pub struct NalaAcquireProgress<'a, B: Backend> {
	config: &'a Config,
	terminal: &'a mut Terminal<B>,
	message: Vec<String>,
	pulse_interval: usize,
	progress: NalaProgressBar,
	ign: String,
	hit: String,
	get: String,
	err: String,
	task: &'a JoinHandle<Result<()>>,
	stop: bool,
}

impl<'a, B: Backend> NalaAcquireProgress<'a, B> {
	/// Returns a new default progress instance.
	pub fn new(
		config: &'a Config,
		terminal: &'a mut Terminal<B>,
		stop: &'a JoinHandle<Result<()>>,
	) -> Self {
		// Try to run a thread here for polling keys.
		// Pulse can then check if it's finished and if it is we quick
		// have it looop and return when there is a key pressed

		let mut progress = Self {
			config,
			terminal,
			message: vec![],
			pulse_interval: 0,
			progress: NalaProgressBar::new(false),
			// TODO: Maybe we should make it configurable.
			ign: config.color.yellow("Ignored").into(),
			hit: config.color.package("No Change").into(),
			get: config.color.blue("Updated").into(),
			err: config.color.red("Error").into(),
			task: stop,
			stop: false,
		};
		// Set Length 1 so ratio cannot panic.
		progress.progress.indicatif.set_length(1);
		// Draw a blank window so it doesn't look weird
		progress.draw();
		progress
	}

	pub fn draw(&mut self) {
		if self.stop {
			return;
		}

		let mut message = vec![];

		if self.message.is_empty() {
			message.push(Span::from("Working...").light_green())
		} else {
			let mut first = true;
			for string in self.message.iter() {
				if first {
					message.push(Span::from(string).light_green());
					first = false;
					continue;
				}
				message.push(Span::from(string).reset().white());
			}
		}

		self.terminal
			.draw(|f| self.progress.render(f, message))
			.unwrap();
	}

	pub fn print(&mut self, msg: String) {
		if self.stop {
			return;
		}
		self.terminal
			.insert_before(1, |buf| {
				Paragraph::new(msg)
					.wrap(Wrap { trim: true })
					.alignment(Alignment::Left)
					.set_style(Style::default().fg(Color::White))
					.render(buf.area, buf);
			})
			.unwrap();
		// Must redraw the terminal after printing
		self.draw();
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
		self.print(format!("{}: {}", self.hit, item.description()))
	}

	/// Called when an Item has started to download
	///
	/// Prints out the short description and the expected size.
	fn fetch(&mut self, item: &ItemDesc) {
		let mut msg = format!("{}:   {}", self.get, item.description());

		let file_size = item.owner().file_size();
		if file_size != 0 {
			msg += &format!(" [{}]", self.progress.unit.str(file_size))
		}

		self.print(msg);
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
		if pending_error() {
			return;
		}

		let msg = if status.fetched_bytes() != 0 {
			self.config
				.color
				.bold(&format!(
					"Fetched {} in {} ({}/s)",
					self.progress.unit.str(status.fetched_bytes()),
					time_str(status.elapsed_time()),
					self.progress.unit.str(status.current_cps()),
				))
				.to_string()
		} else {
			"Nothing to fetch.".to_string()
		};
		self.print(msg);
	}

	/// Called when an Item fails to download.
	///
	/// Print out the ErrorText for the Item.
	fn fail(&mut self, item: &ItemDesc) {
		let mut show_error = self
			.config
			.apt
			.bool("Acquire::Progress::Ignore::ShowErrorText", true);
		let error_text = item.owner().error_text();

		let header = match item.owner().status() {
			ItemState::StatIdle | ItemState::StatDone => {
				if error_text.is_empty() {
					show_error = false;
				}
				&self.ign
			},
			_ => &self.err,
		};

		self.print(format!("{header}: {}", item.description()));

		if show_error {
			self.print(error_text);
		}
	}

	/// Called periodically to provide the overall progress information
	///
	/// Draws the current progress.
	/// Each line has an overall percent meter and a per active item status
	/// meter along with an overall bandwidth and ETA indicator.
	fn pulse(&mut self, status: &AcqTextStatus, owner: &PkgAcquire) {
		self.progress.indicatif.set_length(status.total_bytes());
		self.progress.indicatif.set_position(status.current_bytes());

		if self.task.is_finished() {
			self.terminal.clear().unwrap();
			restore_terminal(true).unwrap();
			self.terminal.show_cursor().unwrap();
			self.stop = true;
			owner.shutdown();
			return;
		}

		// if poll_exit_event().is_ok_and(|poll| poll) {
		// 	self.terminal.clear().unwrap();
		// 	restore_terminal(true).unwrap();
		// 	self.terminal.show_cursor().unwrap();
		// 	self.stop = true;
		// 	owner.shutdown();
		// 	return;
		// }

		let mut string: Vec<String> = vec![];

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

				string.push(work_string);

				string.push(uri);

				// Break only for slim progress
				if !self.config.get_bool("verbose", false) {
					break;
				}
			};
		}

		self.message = string;
		self.draw();
	}
}
