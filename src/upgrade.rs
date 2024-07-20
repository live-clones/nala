use std::io;

use anyhow::Result;
use crossterm::event::{
	self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
	disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Margin, Rect};
use ratatui::style::{Style, Styled, Stylize};
use ratatui::text::{Line, Text};
use ratatui::widgets::{
	Block, BorderType, Borders, Cell, HighlightSpacing, Padding, Paragraph, Row, Scrollbar,
	ScrollbarOrientation, ScrollbarState, Table, TableState,
};
use ratatui::{Frame, Terminal};
use rust_apt::cache::Upgrade;
use rust_apt::new_cache;

use crate::colors::Theme;
use crate::history::{HistoryPackage, Operation};
use crate::util::sudo_check;
use crate::Config;

struct Item {
	align: Alignment,
	style: Style,
	string: String,
}

impl Item {
	fn new(align: Alignment, style: Style, string: String) -> Self {
		Self {
			align,
			style,
			string,
		}
	}

	fn center(style: Style, string: String) -> Self { Self::new(Alignment::Center, style, string) }

	fn right(style: Style, string: String) -> Self { Self::new(Alignment::Right, style, string) }

	fn left(style: Style, string: String) -> Self { Self::new(Alignment::Left, style, string) }

	fn get_cell(&self) -> Cell {
		Cell::from(
			Text::from(self.string.as_str())
				.style(self.style)
				.alignment(self.align),
		)
	}
}

struct App<'a> {
	state: TableState,
	scroll_state: ScrollbarState,
	config: &'a Config,
	items: Vec<Vec<Item>>,
}

impl<'a> App<'a> {
	fn new(config: &'a Config, items: Vec<HistoryPackage>) -> Self {
		let secondary = config.rat_style(Theme::Secondary);
		let primary = config.rat_style(Theme::Regular);

		let scroll_state = ScrollbarState::new(items.len() - 1);
		Self {
			state: TableState::default().with_selected(0),
			scroll_state,
			config,
			items: items
				.into_iter()
				.map(|pkg| {
					let mut items = vec![Item::left(secondary, pkg.name.to_string())];

					if let Some(old_version) = pkg.old_version {
						items.push(Item::center(primary, old_version.clone()));
					}
					items.push(Item::center(primary, pkg.version.to_string()));
					items.push(Item::right(primary, config.unit_str(pkg.size)));
					items
				})
				.collect(),
		}
	}

	fn set_state(&mut self, i: usize) {
		self.state.select(Some(i));
		self.scroll_state = self.scroll_state.position(i);
	}

	pub fn home(&mut self) { self.set_state(0); }

	pub fn end(&mut self) { self.set_state(self.items.len() - 1); }

	pub fn next(&mut self) {
		let i = self.state.selected().unwrap_or_default();
		if i >= self.items.len() - 1 {
			return;
		}
		self.set_state(i + 1);
	}

	pub fn previous(&mut self) {
		let i = self.state.selected().unwrap_or_default();
		if i == 0 {
			return;
		}
		self.set_state(i - 1);
	}

	/// This gets the total length of the entire table
	fn max_row_size(&self) -> u16 {
		self.items
			.iter()
			.map(|vec| vec.iter().map(|item| item.string.len()).sum::<usize>() as u16)
			.max()
			.unwrap_or_default()
	}

	fn contraints(&self) -> Vec<Constraint> {
		// Get the len of the inner vec and
		// generate the constraints by column
		// TODO: Consider storing this in a field?
		(0..self.items[0].len())
			.map(|i| {
				Constraint::Length(
					self.items
						.iter()
						.map(|row| row[i].string.len())
						.max()
						.unwrap_or_default() as u16,
				)
			})
			.collect()
	}
}

pub fn upgrade(config: &Config) -> Result<()> {
	// sudo_check(config)?;
	let cache = new_cache!()?;

	cache.upgrade(Upgrade::FullUpgrade)?;

	let mut upgradable = vec![];
	for pkg in cache.get_changes(true) {
		if pkg.marked_delete() {
			let Some(inst) = pkg.installed() else {
				continue;
			};

			println!("'{inst}' will be REMOVED");
		}

		if let (Some(inst), Some(cand)) = (pkg.installed(), pkg.candidate()) {
			upgradable.push(HistoryPackage {
				name: pkg.name().to_string(),
				version: cand.version().to_string(),
				old_version: Some(inst.version().to_string()),
				size: cand.size(),
				operation: Operation::Upgrade,
				auto_installed: pkg.is_auto_installed(),
			});
		}
	}

	enable_raw_mode()?;
	let mut stdout = io::stdout();
	execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
	let backend = CrosstermBackend::new(stdout);
	let mut terminal = Terminal::new(backend)?;

	// create app and run it
	let app = App::new(config, upgradable);
	let res = run_app(&mut terminal, app, config);

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
	Ok(())
}

fn run_app<B: Backend>(
	terminal: &mut Terminal<B>,
	mut app: App,
	config: &Config,
) -> io::Result<()> {
	loop {
		terminal.draw(|f| ui(f, &mut app, config))?;

		match event::read()? {
			Event::Key(key) => {
				if key.kind == KeyEventKind::Press {
					match key.code {
						KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
						KeyCode::Char('j') | KeyCode::Down => app.next(),
						KeyCode::Char('k') | KeyCode::Up => app.previous(),
						KeyCode::Home => app.home(),
						KeyCode::End => app.end(),
						KeyCode::PageDown => {
							for _ in 0..10 {
								app.next();
							}
						},
						KeyCode::PageUp => {
							for _ in 0..10 {
								app.previous();
							}
						},
						_ => {},
					}
				}
			},
			Event::Mouse(event) => match event.kind {
				MouseEventKind::ScrollDown => app.next(),
				MouseEventKind::ScrollUp => app.previous(),
				_ => {},
			},
			_ => {},
		}
	}
}

fn ui(f: &mut Frame, app: &mut App, config: &Config) {
	let header = format!("  {}  ", "Nala Upgrade").set_style(config.rat_style(Theme::Highlight));

	let block = basic_block(config)
		.title(header)
		.title_alignment(Alignment::Center)
		.padding(Padding::proportional(1));

	let inner = Layout::vertical([Constraint::Min(1), Constraint::Length(2)])
		.flex(Flex::Center)
		.split(block.inner(f.size()));

	let area = Layout::horizontal([
		// Items x Table Column Spacing + 3 for padding
		Constraint::Length(app.max_row_size() + 6 * 5),
		Constraint::Length(3),
	])
	.flex(Flex::Center)
	.split(inner[0]);

	f.render_widget(block, f.size());

	render_table(f, app, area[0], config);

	f.render_widget(basic_block(config), area[1]);

	render_scrollbar(f, app, area[1]);

	let text = "(Esc) quit | (↑) move up | (↓) move down | (→) next color | (←) previous color";
	let info_footer = Paragraph::new(Line::from(text)).centered().block(
		Block::new()
			.borders(Borders::TOP)
			.style(config.rat_style(Theme::Secondary)),
	);

	f.render_widget(info_footer, inner[1]);
}

fn render_table(f: &mut Frame, app: &mut App, area: Rect, config: &Config) {
	let secondary = config.rat_style(Theme::Secondary);
	let white = config.rat_style(Theme::Regular);

	let header = ["Package:", "Old Version:", "New Version:", "Size:"]
		.into_iter()
		.zip(app.items[0].iter())
		.map(|(str, i)| Cell::from(Text::from(str).alignment(i.align)))
		.collect::<Row>()
		.style(white)
		.bottom_margin(1)
		.height(1);

	let t = Table::new(
		app.items
			.iter()
			.map(|vec| Row::from_iter(vec.iter().map(Item::get_cell))),
		app.contraints(),
	)
	.header(header)
	.highlight_style(secondary.reversed())
	.flex(Flex::Center)
	.column_spacing(5)
	.block(basic_block(config))
	.highlight_spacing(HighlightSpacing::Never);

	f.render_stateful_widget(t, area, &mut app.state);
}

fn render_scrollbar(f: &mut Frame, app: &mut App, area: Rect) {
	f.render_stateful_widget(
		Scrollbar::default()
			.orientation(ScrollbarOrientation::VerticalRight)
			.thumb_style(app.config.rat_style(Theme::Primary))
			.track_style(app.config.rat_style(Theme::Secondary))
			.begin_symbol(None)
			.end_symbol(None),
		area.inner(Margin {
			vertical: 1,
			horizontal: 1,
		}),
		&mut app.scroll_state,
	);
}

fn basic_block(config: &Config) -> Block {
	Block::bordered()
		.border_type(BorderType::Rounded)
		.border_style(config.rat_style(Theme::Primary))
}
