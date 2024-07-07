use std::rc::Rc;

use anyhow::Result;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use indicatif::ProgressBar;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, LineGauge, Padding, Paragraph, Widget};
use ratatui::{symbols, Frame, Terminal, TerminalOptions, Viewport};
use rust_apt::util::{time_str, NumSys};

pub struct UnitStr {
	precision: usize,
	base: NumSys,
}

impl UnitStr {
	pub fn new(precision: usize, base: NumSys) -> UnitStr { UnitStr { precision, base } }

	pub fn str(&self, val: u64) -> String {
		let val = val as f64;
		let (num, tera, giga, mega, kilo) = match self.base {
			NumSys::Binary => (1024.0_f64, "TiB", "GiB", "MiB", "KiB"),
			NumSys::Decimal => (1000.0_f64, "TB", "GB", "MB", "KB"),
		};

		let powers = [
			(num.powi(4), tera),
			(num.powi(3), giga),
			(num.powi(2), mega),
			(num, kilo),
		];

		for (divisor, unit) in powers {
			if val > divisor {
				return format!("{:.1$} {unit}", val / divisor, self.precision);
			}
		}
		format!("{val} B")
	}
}

pub struct NalaProgressBar {
	terminal: Terminal<CrosstermBackend<std::io::Stdout>>,
	pub indicatif: ProgressBar,
	pub unit: UnitStr,
	pub msg: Vec<String>,
}

impl NalaProgressBar {
	pub fn new() -> Result<Self> {
		let indicatif = ProgressBar::hidden();
		indicatif.set_length(0);

		enable_raw_mode()?;
		let terminal = Terminal::with_options(
			CrosstermBackend::new(std::io::stdout()),
			TerminalOptions {
				viewport: Viewport::Inline(4),
			},
		)?;

		Ok(Self {
			terminal,
			indicatif,
			unit: UnitStr::new(1, NumSys::Binary),
			msg: vec![],
		})
	}

	pub fn length(&self) -> u64 { self.indicatif.length().unwrap_or_default() }

	// f64 as ceil incase it's less than 1 second we round up to that.
	fn elapsed(&self) -> u64 { self.indicatif.elapsed().as_secs_f64().ceil() as u64 }

	fn ratio(&self) -> f64 { self.indicatif.position() as f64 / self.length() as f64 }

	pub fn clean_up(&mut self) -> Result<()> {
		self.terminal.clear()?;
		disable_raw_mode()?;
		self.terminal.show_cursor()?;
		Ok(())
	}

	pub fn draw(&mut self) -> Result<()> {
		let mut spans = vec![];

		if self.msg.is_empty() {
			spans.push(Span::from("Working...").light_green())
		} else {
			let mut first = true;
			for string in self.msg.iter() {
				if first {
					spans.push(Span::from(string).light_green());
					first = false;
					continue;
				}
				spans.push(Span::from(string).reset().white());
			}
		}
		self.render()?;
		Ok(())
	}

	pub fn print(&mut self, msg: String) -> Result<()> {
		self.terminal.insert_before(1, |buf| {
			Paragraph::new(msg)
				.left_aligned()
				.white()
				.render(buf.area, buf);
		})?;
		// Must redraw the terminal after printing
		self.draw()
	}

	pub fn finished_string(&self) -> String {
		// I've seen this erroneously as 1 before.
		if self.length() > 1 {
			format!(
				"Fetched {} in {} ({}/s)",
				self.unit.str(self.length()),
				time_str(self.elapsed()),
				self.unit.str(self.length() / self.elapsed())
			)
		} else {
			"Nothing to fetch".to_string()
		}
	}

	pub fn render(&mut self) -> Result<()> {
		let mut spans = vec![];

		if self.msg.is_empty() {
			spans.push(Span::from("Working...").light_green())
		} else {
			let mut first = true;
			for string in self.msg.iter() {
				if first {
					spans.push(Span::from(string).light_green());
					first = false;
					continue;
				}
				spans.push(Span::from(string).reset().white());
			}
		}

		let percentage = format!("{:.1}%", self.ratio() * 100.0);
		let current_total = format!(
			"{}/{}",
			self.unit.str(self.indicatif.position()),
			self.unit.str(self.length()),
		);
		let per_sec = format!("{}/s", self.unit.str(self.indicatif.per_sec() as u64));

		let label = if self.indicatif.position() < self.length() {
			Line::from(vec![
				Span::from("Time Remaining:").light_green(),
				Span::from(format!(
					" {}",
					rust_apt::util::time_str(self.indicatif.eta().as_secs())
				)),
			])
		} else {
			Line::from(Span::from("Working...").light_green())
		};

		let bar = LineGauge::default()
			.line_set(symbols::line::THICK)
			.ratio(self.ratio())
			.label(label)
			.style(Style::default().fg(Color::White))
			.gauge_style(Style::default().fg(Color::LightGreen).bg(Color::Red));

		self.terminal
			.draw(|f| render(f, bar, percentage, current_total, per_sec, spans))?;

		Ok(())
	}
}

pub fn render(
	f: &mut Frame,
	bar: LineGauge,
	percentage: String,
	current_total: String,
	per_sec: String,
	spans: Vec<Span>,
) {
	let block = build_block();
	let inner = split_vertical(
		[Constraint::Length(1), Constraint::Length(1)],
		block.inner(f.size()),
	);

	let bar_block = split_horizontal(
		[
			Constraint::Fill(100),
			Constraint::Length(percentage.len() as u16 + 2),
			Constraint::Length(current_total.len() as u16 + 2),
			Constraint::Length(per_sec.len() as u16 + 2),
		],
		inner[1],
	);

	f.render_widget(block, f.size());
	f.render_widget(Paragraph::new(Line::from(spans)), inner[0]);

	f.render_widget(bar, bar_block[0]);
	f.render_widget(get_paragraph(&percentage).blue(), bar_block[1]);
	f.render_widget(get_paragraph(&current_total).light_green(), bar_block[2]);
	f.render_widget(get_paragraph(&per_sec).blue(), bar_block[3]);
}

pub fn get_paragraph(text: &str) -> Paragraph {
	Paragraph::new(text)
		.right_aligned()
}

pub fn build_block<'a>() -> Block<'a> {
	Block::bordered()
		.border_type(BorderType::Rounded)
		.padding(Padding::horizontal(1))
		.style(
			Style::default()
				.fg(Color::LightGreen)
				.add_modifier(Modifier::BOLD),
		)
}

/// Splits a block horizontally with your contraints
pub fn split_horizontal<T>(constraints: T, block: Rect) -> Rc<[Rect]>
where
	T: IntoIterator,
	T::Item: Into<Constraint>,
{
	Layout::default()
		.direction(Direction::Horizontal)
		.constraints(constraints)
		.split(block)
}

/// Splits a block vertically with your contraints
pub fn split_vertical<T>(constraints: T, block: Rect) -> Rc<[Rect]>
where
	T: IntoIterator,
	T::Item: Into<Constraint>,
{
	Layout::default()
		.direction(Direction::Vertical)
		.constraints(constraints)
		.split(block)
}
