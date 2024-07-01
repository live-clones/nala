use std::rc::Rc;

use indicatif::ProgressBar;
use ratatui::layout::{Alignment, Constraint, Direction, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, LineGauge, Paragraph, Wrap};
use ratatui::{symbols, Frame};
use rust_apt::util::NumSys;

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
	pub indicatif: ProgressBar,
	pub unit: UnitStr,
	remaining: bool,
}

impl NalaProgressBar {
	pub fn new(remaining: bool) -> Self {
		let indicatif = ProgressBar::hidden();
		indicatif.set_length(0);

		Self {
			indicatif,
			unit: UnitStr::new(1, NumSys::Binary),
			remaining,
		}
	}

	fn ratio(&self) -> f64 {
		self.indicatif.position() as f64 / self.indicatif.length().unwrap() as f64
	}

	pub fn render(&mut self, f: &mut Frame, msg: Vec<Span>) {
		let block = build_block();
		let inner = split_vertical(
			[Constraint::Length(1), Constraint::Length(1)],
			block.inner(f.size()),
		);
		f.render_widget(block, f.size());

		f.render_widget(Paragraph::new(Line::from(msg)), inner[0]);

		let percentage = format!("{:.1}%", self.ratio() * 100.0);
		let current_total = format!(
			"{}/{}",
			self.unit.str(self.indicatif.position()),
			self.unit.str(self.indicatif.length().unwrap()),
		);
		let per_sec = format!("{}/s ", self.unit.str(self.indicatif.per_sec() as u64));

		let (label, constraints) = if self.remaining {
			(
				Line::from(vec![
					Span::from("Time Remaining:").light_green(),
					Span::from(format!(
						" {}",
						rust_apt::util::time_str(self.indicatif.eta().as_secs())
					)),
				]),
				[
					Constraint::Fill(100),
					Constraint::Length(percentage.len() as u16 + 2),
					Constraint::Length(current_total.len() as u16 + 2),
					Constraint::Length(per_sec.len() as u16 + 2),
				],
			)
		} else {
			(
				Line::from(Span::from("")),
				[
					Constraint::Length(percentage.len() as u16 + 2),
					Constraint::Fill(100),
					Constraint::Length(current_total.len() as u16 + 2),
					Constraint::Length(per_sec.len() as u16 + 2),
				],
			)
		};

		let bar_block = split_horizontal(constraints, inner[1]);

		let bar = LineGauge::default()
			.line_set(symbols::line::THICK)
			.ratio(self.ratio())
			.label(label)
			.style(Style::default().fg(Color::White))
			.gauge_style(Style::default().fg(Color::LightGreen).bg(Color::Red));

		if self.remaining {
			f.render_widget(bar, bar_block[0]);
			f.render_widget(get_paragraph(&percentage).blue(), bar_block[1]);
		} else {
			f.render_widget(
				get_paragraph(&percentage).left_aligned().white(),
				bar_block[0],
			);
			f.render_widget(bar, bar_block[1]);
		}

		f.render_widget(get_paragraph(&current_total).light_green(), bar_block[2]);
		f.render_widget(get_paragraph(&per_sec).blue(), bar_block[3]);
	}
}

pub fn get_paragraph(text: &str) -> Paragraph {
	Paragraph::new(text)
		.wrap(Wrap { trim: true })
		.right_aligned()
}

pub fn build_block<'a>() -> Block<'a> {
	Block::new()
		.borders(Borders::ALL)
		.border_type(BorderType::Rounded)
		.title_alignment(Alignment::Left)
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
		.flex(Flex::SpaceBetween)
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
		// TODO: Figure out how to use flex.
		.flex(Flex::Legacy)
		.direction(Direction::Vertical)
		.constraints(constraints)
		.split(block)
}
