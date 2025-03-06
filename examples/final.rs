use std::{collections::HashMap, time::Duration};

use color_eyre::Result;
use ratatui::{
    DefaultTerminal,
    buffer::Buffer,
    crossterm::event::{self, Event, KeyCode, KeyEventKind},
    layout::{Alignment, Constraint::*, Direction, Layout, Rect},
    style::{Style, Stylize, palette::tailwind},
    symbols::Marker,
    text::Line,
    widgets::{
        Axis, Bar, BarChart, BarGroup, Block, BorderType, Chart, Dataset, GraphType,
        RenderDirection, Row, Sparkline, StatefulWidget, Table, TableState, Widget,
    },
};
use sysinfo::{Disks, Networks, ProcessesToUpdate, System};

fn main() -> Result<()> {
    color_eyre::install()?;
    let terminal = ratatui::init();
    let app_result = App::default().run(terminal);
    ratatui::restore();
    app_result
}

#[derive(Debug)]
struct App {
    running: bool,
    system: System,
    disks: Disks,
    networks: Networks,

    cpu_data: Vec<(f64, f64)>,
    memory_data: Vec<(f64, f64)>,
    disk_data: Vec<DiskData>,
    network_data: HashMap<String, Vec<u64>>,
    table_state: TableState,
}

#[derive(Debug)]
struct DiskData {
    name: String,
    available: u64,
    total: u64,
}

impl Default for App {
    fn default() -> Self {
        Self {
            running: true,
            system: System::new_all(),
            disks: Disks::new_with_refreshed_list(),
            networks: Networks::new_with_refreshed_list(),
            cpu_data: Vec::new(),
            memory_data: Vec::new(),
            disk_data: Vec::new(),
            network_data: HashMap::new(),
            table_state: TableState::default(),
        }
    }
}

impl App {
    fn run(mut self, mut terminal: DefaultTerminal) -> Result<()> {
        self.update_disk_data();
        self.table_state.select_first();
        while self.running {
            terminal.draw(|frame| {
                frame.render_widget(&mut self, frame.area());
                self.update(frame.count());
            })?;
            self.handle_events()?;
        }
        Ok(())
    }

    fn update_disk_data(&mut self) {
        self.disks.refresh(true);
        self.disk_data = self
            .disks
            .list()
            .iter()
            .map(|disk| DiskData {
                name: disk.name().to_string_lossy().to_string(),
                available: disk.available_space(),
                total: disk.total_space(),
            })
            .collect();
    }

    fn update_network_data(&mut self) {
        self.networks.refresh(true);
        for (interface_name, network) in &self.networks {
            self.network_data
                .entry(interface_name.clone())
                .or_default()
                .push(network.packets_received() + network.packets_transmitted());
        }
    }

    fn update(&mut self, frame_count: usize) {
        if frame_count % 30 == 0 {
            self.system.refresh_processes(ProcessesToUpdate::All, true);
        }

        self.system.refresh_cpu_all();
        let cpu_usage = self.system.global_cpu_usage();
        self.cpu_data.push((frame_count as f64, cpu_usage as f64));

        self.system.refresh_memory();
        let memory_usage = self.system.used_memory();
        self.memory_data
            .push((frame_count as f64, memory_usage as f64));

        self.update_network_data();
    }

    fn handle_events(&mut self) -> Result<()> {
        let timeout = Duration::from_secs_f32(1.0 / 60.0);
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') => self.running = false,
                        KeyCode::Down | KeyCode::Char('j') => self.table_state.select_next(),
                        KeyCode::Up | KeyCode::Char('k') => self.table_state.select_previous(),
                        _ => {}
                    }
                }
            }
        }
        Ok(())
    }
}

impl Widget for &mut App {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        let layout = Layout::vertical([Length(1), Min(0)]);
        let [header_area, main_area] = layout.areas(area);

        let [first, second, third] =
            Layout::vertical([Percentage(25), Fill(1), Fill(1)]).areas(main_area);
        let [disk_area, memory_area] = Layout::horizontal([Percentage(30), Fill(1)]).areas(second);
        let [network_area, process_area] = Layout::horizontal([Fill(1); 2]).areas(third);

        self.render_header(header_area, buffer);
        self.render_cpu(first, buffer);
        self.render_disk(disk_area, buffer);
        self.render_memory(memory_area, buffer);
        self.render_network(network_area, buffer);
        self.render_process(process_area, buffer);
    }
}

impl App {
    fn render_header(&self, area: Rect, buf: &mut Buffer) {
        Line::from("Ratatop")
            .alignment(Alignment::Center)
            .fg(tailwind::BLUE.c200)
            .bg(tailwind::GRAY.c800)
            .bold()
            .render(area, buf);
    }

    fn render_cpu(&self, area: Rect, buf: &mut Buffer) {
        let current_percentage = self.cpu_data.last().map(|(_, v)| v).unwrap_or(&0.0);
        let current_percentage_line =
            format!("{:.2}%", current_percentage).fg(match current_percentage {
                0.0..=50.0 => tailwind::GREEN.c400,
                50.0..=80.0 => tailwind::YELLOW.c300,
                _ => tailwind::RED.c600,
            });

        let datasets = vec![
            Dataset::default()
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(tailwind::GREEN.c400)
                .data(&self.cpu_data),
        ];

        let x_axis = Axis::default()
            .title(current_percentage_line)
            .bounds([0.0, self.cpu_data.len() as f64]);

        let y_axis = Axis::default()
            .bounds([0.0, 100.0])
            .labels(vec![
                "0%".fg(tailwind::GREEN.c400),
                "50%".fg(tailwind::YELLOW.c300),
                "100%".fg(tailwind::RED.c600),
            ])
            .style(tailwind::GRAY.c600);

        let chart = Chart::new(datasets)
            .block(pane("CPU").title_alignment(Alignment::Right))
            .style(Style::new().bg(tailwind::GRAY.c900))
            .x_axis(x_axis)
            .y_axis(y_axis);

        chart.render(area, buf);
    }

    fn render_disk(&self, area: Rect, buf: &mut Buffer) {
        let bars = self
            .disk_data
            .iter()
            .map(|disk| {
                let name = disk.name.rsplit('/').next().unwrap().to_string();
                let percent = (disk.available as f64 / disk.total as f64 * 100.0) as u64;
                let style = match percent {
                    0..=50 => tailwind::GREEN.c400,
                    51..=80 => tailwind::YELLOW.c300,
                    _ => tailwind::RED.c600,
                };
                Bar::default()
                    .label(name.fg(tailwind::BLUE.c100).into())
                    .value(percent)
                    .style(style)
            })
            .collect::<Vec<_>>();

        let data = BarGroup::default().bars(&bars);
        BarChart::default()
            .block(pane("Disks"))
            .style(Style::new().bg(tailwind::GRAY.c900))
            .direction(Direction::Horizontal)
            .data(data)
            .bar_gap(1)
            .bar_width(1)
            .bar_style(Style::new().on_black())
            .render(area, buf);
    }

    fn render_memory(&self, area: Rect, buf: &mut Buffer) {
        let current_percentage =
            self.system.used_memory() as f64 / self.system.total_memory() as f64 * 100.0;
        let current_percentage_line =
            format!("{:.2}%", current_percentage).fg(match current_percentage {
                0.0..=50.0 => tailwind::GREEN.c400,
                50.0..=80.0 => tailwind::YELLOW.c300,
                _ => tailwind::RED.c600,
            });

        let datasets = vec![
            Dataset::default()
                .name(current_percentage_line)
                .marker(Marker::Bar)
                .graph_type(GraphType::Line)
                .style(tailwind::BLUE.c400)
                .data(&self.memory_data),
        ];
        let x_axis = Axis::default().bounds([0.0, self.memory_data.len() as f64]);
        let y_axis = Axis::default().bounds([0.0, self.system.total_memory() as f64]);
        let chart = Chart::new(datasets)
            .style(Style::new().bg(tailwind::GRAY.c900))
            .block(pane("Memory"))
            .x_axis(x_axis)
            .y_axis(y_axis);

        chart.render(area, buf);
    }

    fn render_network(&self, area: Rect, buf: &mut Buffer) {
        let block = pane("Network");
        let inner = block.inner(area);
        block.render(area, buf);

        let mut network_data = self.network_data.iter().collect::<Vec<_>>();
        network_data.sort_by(|(name1, _), (name2, _)| name1.cmp(name2));
        let longest_name = network_data
            .iter()
            .map(|(name, _)| name.len())
            .max()
            .unwrap_or(0);
        let layout = Layout::horizontal([Length(longest_name as u16), Fill(1)]).spacing(1);
        let [name_area, data_area] = layout.areas(inner);

        for (((name, data), name_row), data_row) in network_data
            .into_iter()
            .zip(name_area.rows())
            .zip(data_area.rows())
        {
            Line::from(name.clone())
                .fg(tailwind::BLUE.c200)
                .render(name_row, buf);
            Sparkline::default()
                .data(data.iter().rev().take(data_area.width as usize).rev())
                .direction(RenderDirection::LeftToRight)
                .style(tailwind::GREEN.c400)
                .render(data_row, buf);
        }
    }

    fn render_process(&mut self, area: Rect, buf: &mut Buffer) {
        let header = Row::new(vec!["Pid", "Cmd", "CPU%", "Mem%"]).style(tailwind::YELLOW.c200);
        let widths = [Length(10), Fill(2), Fill(1), Fill(1)];
        let mut rows = Vec::new();
        for (pid, process) in self.system.processes() {
            let row = vec![
                pid.to_string(),
                process.name().to_string_lossy().to_string(),
                format!("{:.2}", process.cpu_usage()),
                format!(
                    "{:.2}",
                    process.memory() as f64 / self.system.total_memory() as f64 * 100.0
                ),
            ];
            rows.push(row);
        }
        rows.sort_by(|a, b| {
            a[2].parse::<f64>()
                .unwrap_or_default()
                .partial_cmp(&b[2].parse::<f64>().unwrap_or_default())
                .unwrap()
                .reverse()
        });

        let table = Table::new(
            rows.into_iter()
                .map(|v| Row::new(v).fg(tailwind::GRAY.c400))
                .collect::<Vec<_>>(),
            widths,
        )
        .header(header)
        .style(tailwind::GRAY.c900)
        .row_highlight_style(Style::new().bg(tailwind::GRAY.c800).fg(tailwind::BLUE.c200))
        .highlight_symbol("> ")
        .block(pane("Processes"));

        StatefulWidget::render(table, area, buf, &mut self.table_state);
    }
}

/// Creates a bordered block with a title.
fn pane(title: &str) -> Block {
    let title = Line::from_iter([
        "┤ ".fg(tailwind::GRAY.c700),
        title.fg(tailwind::BLUE.c200),
        " ├".fg(tailwind::GRAY.c700),
    ]);
    Block::bordered()
        .border_type(BorderType::Rounded)
        .title(title)
        .title_alignment(Alignment::Left)
        .border_style(tailwind::GRAY.c700)
        .style(Style::new().bg(tailwind::GRAY.c900))
}
