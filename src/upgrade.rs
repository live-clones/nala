use std::collections::{BTreeMap, HashMap};
use std::io;

use ansi_to_tui::IntoText;
use anyhow::Result;
use crossterm::event::{
	self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
	disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Constraint::{Length, Max, Min};
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Margin, Rect};
use ratatui::style::{Style, Styled};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
	Block, BorderType, Cell, HighlightSpacing, Padding, Paragraph, Row, Scrollbar,
	ScrollbarOrientation, ScrollbarState, StatefulWidget, Table, TableState, Tabs, Widget, Wrap,
};
use ratatui::Terminal;
use rust_apt::cache::Upgrade;
use rust_apt::util::DiskSpace;
use rust_apt::{new_cache, Cache, Version};

use crate::colors::Theme;
use crate::history::{HistoryPackage, Operation};
use crate::show::{build_regex, show_version};
use crate::util::sudo_check;
use crate::Config;

#[derive(Debug)]
struct Item {
	align: Alignment,
	style: Style,
	string: String,
	old_version: Option<String>,
}

impl Item {
	fn new(align: Alignment, style: Style, string: String, old_version: Option<String>) -> Self {
		Self {
			align,
			style,
			string,
			old_version,
		}
	}

	fn center(style: Style, string: String, old_version: Option<String>) -> Self {
		Self::new(Alignment::Center, style, string, old_version)
	}

	fn right(style: Style, string: String) -> Self {
		Self::new(Alignment::Right, style, string, None)
	}

	fn left(style: Style, string: String) -> Self {
		Self::new(Alignment::Left, style, string, None)
	}

	fn get_cell(&self, config: &Config) -> Cell {
		let line = if let Some(old) = &self.old_version {
			version_diff(config, &old, self.string.to_string())
		} else {
			Line::from(self.string.as_str())
		}
		.style(self.style)
		.alignment(self.align);

		Cell::from(Text::from(line))
	}
}

fn version_diff<'a>(config: &Config, old: &'a str, new: String) -> Line<'a> {
	// Check for just revision change first.
	if let (Some(old_ver), Some(new_ver)) = (old.rsplit_once("-"), new.rsplit_once("-")) {
		// If there isn't a revision these shouldn't ever match
		// If they do match then only the revision has changed
		if old_ver.0 == new_ver.0 {
			return Line::from_iter([
				Span::raw(new_ver.0.to_string()),
				Span::raw("-"),
				Span::raw(new_ver.1.to_string()).style(config.rat_style(Theme::Notice)),
			]);
			// return format!("{}-{}", new_ver.0, config.color(Theme::Notice,
			// new_ver.0));
		}
	}

	let (old_ver, new_ver) = (
		old.split(".").collect::<Vec<_>>(),
		new.split(".").collect::<Vec<_>>(),
	);

	let mut start_color = 0;
	for (i, section) in old_ver.iter().enumerate() {
		if i > new_ver.len() - 1 {
			break;
		}

		if section != &new_ver[i] {
			start_color = i;
			break;
		}
	}

	let mut new = vec![];
	for (i, str) in new_ver.iter().enumerate() {
		if i >= start_color {
			new.push(Span::from(str.to_string()).style(config.rat_style(Theme::Notice)));
		} else {
			new.push(Span::from(str.to_string()));
		}

		if i < new_ver.len() - 1 {
			new.push(Span::from("."));
		}
	}
	return Line::from_iter(new);

	// use ansi-to-tui for this instead of Rat

	// No Rat
	// new_ver
	// 	.iter()
	// 	.enumerate()
	// 	.map(|(i, str)| {
	// 		if i >= start_color {
	// 			config.color(Theme::Notice, str)
	// 		} else {
	// 			str.to_string()
	// 		}
	// 	})
	// 	.collect::<Vec<_>>()
	// 	.join(".")
}

struct SummaryPkg<'a> {
	version: Version<'a>,
	// TODO: Use this to show both versions??
	old_version: Option<Version<'a>>,
	items: Vec<Item>,
}

impl<'a> SummaryPkg<'a> {
	pub fn new(
		config: &Config,
		op: Operation,
		version: Version<'a>,
		old_version: Option<Version<'a>>,
	) -> Self {
		let secondary = config.rat_style(op.theme());
		let primary = config.rat_style(Theme::Regular);
		let mut items = vec![Item::left(secondary, version.parent().name().to_string())];

		if let Some(old) = &old_version {
			items.push(Item::center(primary, old.version().to_string(), None));
			items.push(Item::center(
				primary,
				version.version().to_string(),
				Some(old.version().to_string()),
			));
		} else {
			items.push(Item::center(primary, version.version().to_string(), None));
		}
		items.push(Item::right(primary, config.unit_str(version.size())));

		Self { version, old_version, items }
	}

	pub fn render_show(
		&self,
		terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
		config: &Config,
	) -> Result<()> {
		let pkg = self.version.parent();
		let pacstall_regex = build_regex(r#"_remoterepo="(.*?)""#)?;
		let url_regex = build_regex("(https?://.*?/.*?/)")?;
		// Maybe we will show both versions if available?
		let show = show_version(config, &pkg, &self.version, &pacstall_regex, &url_regex);
		terminal.clear()?;

		let mut lines: Vec<Text> = vec![];
		for (head, info) in &show {
			let mut split = info.split("\n");
			if let Some(first) = split.next() {
				lines.push(
					format!("{}: {first}", config.color(Theme::Highlight, head)).into_text()?,
				);
				for line in split {
					let line = line.to_string();
					lines.push(line.into_text()?)
				}
			}
		}

		loop {
			terminal.draw(|f| {
				let header =
					format!("  {}  ", "Nala Upgrade").set_style(config.rat_style(Theme::Highlight));

				let block = basic_block(config)
					.title(header)
					.title_alignment(Alignment::Center)
					.padding(Padding::horizontal(1));

				let inner = block.inner(f.size());

				let constraints = lines
					.iter()
					.map(|line| Length((line.width() as f32 / inner.width as f32).ceil() as u16))
					.collect::<Vec<_>>();

				let layout = Layout::vertical(constraints)
					.split(block.inner(f.size()));

				f.render_widget(block, f.size());
				for (i, line) in lines.iter().enumerate() {
					f.render_widget(
						Paragraph::new(line.clone()).wrap(Wrap::default()),
						layout[i],
					)
				}
			})?;

			match event::read()? {
				Event::Key(key) => {
					if key.kind == KeyEventKind::Press {
						match key.code {
							KeyCode::Char('q') | KeyCode::Esc => break Ok(()),
							_ => {},
						}
					}
				},
				_ => {},
			}
		}
	}
}

struct App<'a> {
	state: TableState,
	scroll_state: ScrollbarState,
	config: &'a Config,
	items: Vec<SummaryPkg<'a>>,
}

impl<'a> App<'a> {
	fn new(config: &'a Config, items: Vec<SummaryPkg<'a>>) -> Self {
		let scroll_state = ScrollbarState::new(items.len() - 1);
		Self {
			state: TableState::default().with_selected(0),
			scroll_state,
			config,
			items,
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

	fn render_table(&mut self, area: Rect, buf: &mut Buffer) {
		let highlight = self.config.rat_style(Theme::Primary);
		let white = self.config.rat_style(Theme::Regular);

		let headers = if self.items.len() > 3 {
			vec!["Package:", "Old Version:", "New Version:", "Size:"]
		} else {
			vec!["Package:", "Version:", "Size:"]
		};

		let header = headers
			.into_iter()
			.zip(self.items[0].items.iter())
			.map(|(str, i)| Cell::from(Text::from(str).alignment(i.align)))
			.collect::<Row>()
			.style(white);

		let mut constraints = vec![];
		for i in 0..self.items[0].items.len() {
			constraints.push(
				self.items
					.iter()
					.map(|item| item.items[i].string.len())
					.max()
					.unwrap_or_default() as u16,
			)
		}

		let t = Table::new(
			self.items
				.iter()
				.map(|vec| Row::from_iter(vec.items.iter().map(|item| item.get_cell(self.config)))),
			constraints,
		)
		.header(header)
		.highlight_style(highlight)
		.flex(Flex::SpaceAround)
		.block(basic_block(self.config))
		.highlight_spacing(HighlightSpacing::Never);

		StatefulWidget::render(t, area, buf, &mut self.state);
	}
}

impl<'a> StatefulWidget for &mut App<'a> {
	type State = TableState;

	fn render(self, area: Rect, buf: &mut Buffer, _: &mut Self::State) {
		let table_area = Layout::horizontal([Constraint::Min(0), Length(3)]).split(area);

		self.render_table(table_area[0], buf);

		basic_block(self.config).render(table_area[1], buf);

		StatefulWidget::render(
			Scrollbar::default()
				.orientation(ScrollbarOrientation::VerticalRight)
				.thumb_style(self.config.rat_style(Theme::Primary))
				.track_style(self.config.rat_style(Theme::Secondary))
				.begin_symbol(None)
				.end_symbol(None),
			table_area[1].inner(Margin {
				vertical: 1,
				horizontal: 1,
			}),
			buf,
			&mut self.scroll_state,
		);
	}
}

struct SummaryTab<'a> {
	fake_state: TableState,
	config: &'a Config,
	pkg_set: BTreeMap<Operation, App<'a>>,
	// Array first is the header, second is string.
	download_size: Option<Vec<String>>,
	disk_space: Vec<String>,
	i: usize,
	tabs: Vec<Operation>,
}

impl<'a> SummaryTab<'a> {
	fn new(
		cache: &Cache,
		config: &'a Config,
		pkg_set: HashMap<Operation, Vec<SummaryPkg<'a>>>,
	) -> Self {
		let pkg_set = pkg_set
			.into_iter()
			.map(|(op, set)| (op, App::new(config, set)))
			.collect();

		let size = cache.depcache().download_size();
		let download_size = if size > 0 {
			Some(vec![
				"Total download size:".to_string(),
				config.unit_str(size),
			])
		} else {
			None
		};

		let disk_space = match cache.depcache().disk_size() {
			DiskSpace::Require(num) => {
				vec!["Disk space required:".to_string(), config.unit_str(num)]
			},
			DiskSpace::Free(num) => vec!["Disk space to free:".to_string(), config.unit_str(num)],
		};

		let mut tabs = Self {
			fake_state: TableState::default().with_selected(0),
			config,
			pkg_set,
			download_size,
			disk_space,
			i: 0,
			tabs: Operation::to_vec(),
		};

		for (i, tab) in tabs.tabs.iter().enumerate() {
			if tabs.pkg_set.contains_key(tab) {
				tabs.i = i;
				break;
			}
		}

		tabs
	}

	pub fn current_tab(&self) -> Operation { self.tabs[self.i] }

	pub fn current(&self) -> &App<'a> { self.pkg_set.get(&self.current_tab()).unwrap() }

	pub fn current_mut(&mut self) -> &mut App<'a> {
		self.pkg_set.get_mut(&self.current_tab()).unwrap()
	}

	fn real_next(&mut self) -> bool {
		let max = self.tabs.len() - 1;
		if self.i >= max {
			self.i = max;
			return false;
		}
		self.i = self.i + 1;
		true
	}

	fn real_previous(&mut self) -> bool {
		if self.i == 0 {
			return false;
		}
		self.i = self.i - 1;
		true
	}

	pub fn next_tab(&mut self) {
		let i = self.i;
		self.real_next();
		while !self.pkg_set.contains_key(&self.tabs[self.i]) {
			if !self.real_next() {
				self.i = i;
				return;
			}
		}
	}

	pub fn previous_tab(&mut self) {
		let i = self.i;
		self.real_previous();
		while !self.pkg_set.contains_key(&self.tabs[self.i]) {
			if !self.real_previous() {
				self.i = i;
				return;
			}
		}
	}

	fn render_tabs(&self, area: Rect, buf: &mut Buffer) {
		let titles: Vec<&str> = self
			.tabs
			.iter()
			.filter_map(
				|op| {
					if self.pkg_set.contains_key(&op) {
						Some(op.as_str())
					} else {
						None
					}
				},
			)
			.collect();

		let tab_size = titles.iter().map(|s| s.len()).sum::<usize>() + titles.len() + 1;
		let new_area = Layout::horizontal([tab_size as u16]).split(area);

		let position = self
			.pkg_set
			.iter()
			.position(|(op, _)| op == &self.current_tab())
			.unwrap();

		Tabs::new(titles)
			.highlight_style(self.config.rat_style(Theme::Primary))
			.select(position)
			.padding("", "")
			.divider(" ")
			.render(new_area[0], buf);
	}

	fn run(&mut self) -> Result<()> {
		enable_raw_mode()?;
		let mut stdout = io::stdout();
		execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
		let backend = CrosstermBackend::new(stdout);
		let mut terminal = Terminal::new(backend)?;

		// TODO: Make a function so that we can return a result.
		let res: Result<()> = loop {
			let mut fake_state = self.fake_state.clone();

			terminal.draw(|frame| {
				frame.render_stateful_widget(&mut *self, frame.size(), &mut fake_state)
			})?;

			match event::read()? {
				Event::Key(key) => {
					if key.kind == KeyEventKind::Press {
						match key.code {
							KeyCode::Char('q') | KeyCode::Esc => break Ok(()),
							KeyCode::Char('l') | KeyCode::Right => self.next_tab(),
							KeyCode::Char('h') | KeyCode::Left => self.previous_tab(),
							KeyCode::Char('j') | KeyCode::Down => self.current_mut().next(),
							KeyCode::Char('k') | KeyCode::Up => self.current_mut().previous(),
							KeyCode::Home => self.current_mut().home(),
							KeyCode::End => self.current_mut().end(),
							KeyCode::PageDown => {
								for _ in 0..10 {
									self.current_mut().next();
								}
							},
							KeyCode::PageUp => {
								for _ in 0..10 {
									self.current_mut().previous();
								}
							},
							KeyCode::Enter => {
								let app = self.current();
								if let Some(i) = app.state.selected() {
									app.items[i].render_show(&mut terminal, self.config)?;
								}
							},
							_ => {},
						}
					}
				},
				Event::Mouse(event) => match event.kind {
					MouseEventKind::ScrollDown => self.current_mut().next(),
					MouseEventKind::ScrollUp => self.current_mut().previous(),
					_ => {},
				},
				_ => {},
			}
		};

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
}

impl<'a> StatefulWidget for &mut SummaryTab<'a> {
	type State = TableState;

	fn render(self, area: Rect, buf: &mut Buffer, _: &mut Self::State) {
		let header =
			format!("  {}  ", "Nala Upgrade").set_style(self.config.rat_style(Theme::Highlight));

		let block = basic_block(self.config)
			.title(header)
			.title_alignment(Alignment::Center)
			.padding(Padding::horizontal(1));

		let mut summary = vec![];
		for op in Operation::to_vec().iter() {
			let Some(set) = self.pkg_set.get(op) else {
				continue;
			};

			summary.push(vec![format!("{op}:"), format!("{} Pkgs", set.items.len())]);
		}

		summary.push(vec![]);
		if let Some(array) = &self.download_size {
			summary.push(array.clone());
		}
		summary.push(self.disk_space.clone());

		let mut header_len = 0;
		let mut size_len = 0;
		for vec in &summary {
			if vec.is_empty() {
				continue;
			}
			if vec[0].len() > header_len {
				header_len = vec[0].len()
			}
			if vec[1].len() > size_len {
				size_len = vec[1].len()
			}
		}

		let [tab, table, footer] =
			Layout::vertical([Length(1), Min(0), Length(summary.len() as u16)])
				.flex(Flex::Center)
				.areas(block.inner(area));

		block.render(area, buf);

		self.render_tabs(tab, buf);

		self.pkg_set
			.get_mut(&self.current_tab())
			.unwrap()
			.render(table, buf, &mut self.fake_state);

		let text = [
			"(↑) move up | (↓) move down",
			"(→) next tab | (←) previous tab",
			"(q) quit | (y) start upgrade",
		];

		let [summary_area, info_area] = Layout::horizontal([
			Max((header_len + size_len) as u16),
			Max(text.iter().map(|s| s.len()).max().unwrap_or_default() as u16),
		])
		.flex(Flex::SpaceAround)
		.areas(footer);

		let t = Table::new(
			summary.iter().map(|a| {
				if a.is_empty() {
					Row::new(a.clone())
				} else {
					Row::new([
						Cell::from(Text::from(a[0].as_str())),
						Cell::from(Text::from(a[1].as_str()).right_aligned()),
					])
				}
			}),
			[Length(header_len as u16), Length(size_len as u16)],
		);
		Widget::render(t, summary_area, buf);

		Paragraph::new(Text::from_iter(text))
			.centered()
			.style(self.config.rat_style(Theme::Secondary))
			.wrap(Wrap::default())
			.render(info_area, buf);
	}
}

pub fn upgrade(config: &Config) -> Result<()> {
	// sudo_check(config)?;
	let cache = new_cache!()?;

	cache.upgrade(Upgrade::FullUpgrade)?;

	let mut pkg_set: HashMap<Operation, Vec<SummaryPkg>> = HashMap::new();

	for pkg in cache.get_changes(true) {
		if pkg.marked_delete() {
			let Some(inst) = pkg.installed() else {
				continue;
			};

			println!("'{inst}' will be REMOVED");
		}

		if pkg.marked_install() {
			if let Some(cand) = pkg.install_version() {
				pkg_set
					.entry(Operation::Install)
					.or_default()
					.push(SummaryPkg::new(config, Operation::Install, cand, None));
			}
		}

		if let (Some(inst), Some(cand)) = (pkg.installed(), pkg.candidate()) {
			pkg_set
				.entry(Operation::Upgrade)
				.or_default()
				.push(SummaryPkg::new(
					config,
					Operation::Upgrade,
					cand,
					Some(inst),
				));
		}
	}

	enable_raw_mode()?;
	let mut stdout = io::stdout();
	execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
	let backend = CrosstermBackend::new(stdout);
	let mut terminal = Terminal::new(backend)?;

	// create app and run it
	let mut summary = SummaryTab::new(&cache, config, pkg_set);

	let res = summary.run();

	// for (_operation, set) in pkg_set {
	// 	let app = App::new(config, set);
	// 	let _res = run_app(&mut terminal, app, config);
	// }
	// let app = App::new(config, upgradable);
	// let res = run_app(&mut terminal, app, config);

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

fn basic_block(config: &Config) -> Block {
	Block::bordered()
		.border_type(BorderType::Thick)
		.border_style(config.rat_style(Theme::Primary))
}
