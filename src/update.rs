use anyhow::Result;
use ratatui::backend::Backend;
use ratatui::style::Stylize;
use ratatui::text::Span;
use ratatui::widgets::{Paragraph, Widget};
use ratatui::Terminal;
use rust_apt::progress::{AcquireProgress, DynAcquireProgress};
use rust_apt::raw::{AcqTextStatus, ItemDesc, ItemState, PkgAcquire};
use rust_apt::{new_cache, PackageSort};
use tokio::sync::mpsc;

use crate::config::Config;
use crate::tui;
use crate::util::{init_terminal, restore_terminal};

pub enum Message {
	Print(String),
	Messages(Vec<String>),
	UpdatePosition((u64, u64)),
	Fetched((String, u64)),
}

pub struct App<B: Backend> {
	terminal: Terminal<B>,
	progress: tui::NalaProgressBar,
	message: Vec<String>,
}

impl<B: Backend> App<B> {
	pub fn new(terminal: Terminal<B>) -> App<B> {
		App {
			terminal,
			progress: tui::NalaProgressBar::new(),
			message: vec![],
		}
	}

	pub fn clean_up(&mut self) -> Result<()> {
		self.terminal.clear()?;
		restore_terminal(true)?;
		self.terminal.show_cursor()?;
		Ok(())
	}

	pub fn draw(&mut self) -> Result<()> {
		let mut spans = vec![];

		if self.message.is_empty() {
			spans.push(Span::from("Working...").light_green())
		} else {
			let mut first = true;
			for string in self.message.iter() {
				if first {
					spans.push(Span::from(string).light_green());
					first = false;
					continue;
				}
				spans.push(Span::from(string).reset().white());
			}
		}

		self.terminal.draw(|f| self.progress.render(f, spans))?;
		Ok(())
	}

	pub fn print(&mut self, msg: String) -> Result<()> {
		self.terminal.insert_before(1, |buf| {
			Paragraph::new(msg)
				.left_aligned()
				.white()
				.render(buf.area, buf);
		})?;
		self.draw()?;
		Ok(())
	}
}

/// The function just runs apt's update and is designed to go into
/// it's own little thread.
pub async fn update_thread(acquire: NalaAcquireProgress) -> Result<()> {
	let cache = new_cache!()?;
	cache.update(&mut AcquireProgress::new(acquire))?;
	Ok(())
}

#[tokio::main]
pub async fn update(config: &Config) -> Result<()> {
	// Setup channel to talk between threads
	let (tx, mut rx) = mpsc::unbounded_channel();
	// Setup the acquire struct and send it to the update thread
	let acquire = NalaAcquireProgress::new(config, tx);
	let task = tokio::task::spawn(update_thread(acquire));

	let mut app = App::new(init_terminal(true)?);

	while let Some(msg) = rx.recv().await {
		match msg {
			Message::UpdatePosition((total, current)) => {
				app.progress.indicatif.set_length(total);
				app.progress.indicatif.set_position(current);
			},
			Message::Print(msg) => {
				app.print(msg)?;
			},
			Message::Fetched((msg, file_size)) => {
				app.print(if file_size > 0 {
					format!("{msg} [{}]", app.progress.unit.str(file_size))
				} else {
					msg
				})?;
			},
			Message::Messages(msg) => {
				app.message = msg;
				app.draw()?;
			},
		}

		// Exit immedately.
		// This is the only way to stop apt's update
		if tui::poll_exit_event()? {
			app.clean_up()?;
			std::process::exit(1);
		}
	}

	app.clean_up()?;

	task.await??;

	println!("{}", config.color.bold(&app.progress.finished_string()));

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

	Ok(())
}

/// AptAcquireProgress is the default struct for the update method on the cache.
///
/// This struct mimics the output of `apt update`.
pub struct NalaAcquireProgress {
	apt_config: rust_apt::config::Config,
	pulse_interval: usize,
	ign: String,
	hit: String,
	get: String,
	err: String,
	tx: mpsc::UnboundedSender<Message>,
}

impl NalaAcquireProgress {
	/// Returns a new default progress instance.
	pub fn new(config: &Config, tx: mpsc::UnboundedSender<Message>) -> Self {
		Self {
			apt_config: rust_apt::config::Config::new(),
			pulse_interval: 0,
			// TODO: Maybe we should make it configurable.
			ign: config.color.yellow("Ignored").into(),
			hit: config.color.package("No Change").into(),
			get: config.color.blue("Updated").into(),
			err: config.color.red("Error").into(),
			tx,
		}
	}
}

impl DynAcquireProgress for NalaAcquireProgress {
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
		self.tx
			.send(Message::Print(format!(
				"{}: {}",
				self.hit,
				item.description()
			)))
			.unwrap();
	}

	/// Called when an Item has started to download
	///
	/// Prints out the short description and the expected size.
	fn fetch(&mut self, item: &ItemDesc) {
		self.tx
			.send(Message::Fetched((
				format!("{}:   {}", self.get, item.description()),
				item.owner().file_size(),
			)))
			.unwrap();
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
	fn stop(&mut self, _: &AcqTextStatus) {}

	/// Called when an Item fails to download.
	///
	/// Print out the ErrorText for the Item.
	fn fail(&mut self, item: &ItemDesc) {
		let mut show_error = self
			.apt_config
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

		self.tx
			.send(Message::Print(format!("{header}: {}", item.description())))
			.unwrap();

		if show_error {
			self.tx.send(Message::Print(error_text)).unwrap();
		}
	}

	/// Called periodically to provide the overall progress information
	///
	/// Draws the current progress.
	/// Each line has an overall percent meter and a per active item status
	/// meter along with an overall bandwidth and ETA indicator.
	fn pulse(&mut self, status: &AcqTextStatus, owner: &PkgAcquire) {
		self.tx
			.send(Message::UpdatePosition((
				status.total_bytes(),
				status.current_bytes(),
			)))
			.unwrap();

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
				break;
			};
		}

		self.tx.send(Message::Messages(string)).unwrap();
	}
}
