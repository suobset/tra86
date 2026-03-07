use chrono::{DateTime, Local};
use egui::{Color32, Key, RichText, ScrollArea, TextEdit};
use tra86_core::{
    Address, Breakpoint, BreakpointLocation, DisassemblyLine, FrameState, RegisterBank,
    RegisterDiff, StopReason, ThreadState, TraceEvent,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendChoice {
    Mock,
    Lldb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BottomTab {
    Stack,
    Memory,
    Breakpoints,
    Trace,
    Analysis,
    Output,
}

#[derive(Debug, Clone)]
pub enum UiEvent {
    SetBackend(BackendChoice),
    OpenExecutablePicker,
    Launch,
    Attach,
    Continue,
    Pause,
    StepInto,
    StepOver,
    StepOut,
    Restart,
    Stop,
    Refresh,
    ToggleBreakpoint(Address),
    RemoveBreakpoint(u64),
    JumpMemory(Address),
    JumpDisassembly(Address),
    ClearOutput,
    ClearTrace,
    SetBottomTab(BottomTab),
    JumpToCurrentInstruction,
    ShowAbout,
    Quit,
}

#[derive(Debug, Clone)]
pub struct UiModel {
    pub backend_choice: BackendChoice,
    pub executable_path: String,
    pub launch_args: String,
    pub attach_pid: String,
    pub status_line: String,
    pub stop_reason: StopReason,
    pub threads: Vec<ThreadState>,
    pub frames: Vec<FrameState>,
    pub function_rows: Vec<String>,
    pub disassembly: Vec<DisassemblyLine>,
    pub registers: Option<RegisterBank>,
    pub register_diffs: Vec<RegisterDiff>,
    pub breakpoints: Vec<Breakpoint>,
    pub trace: Vec<TraceEvent>,
    pub memory_map_lines: Vec<String>,
    pub memory_base_input: String,
    pub memory_bytes: Vec<u8>,
    pub analysis_lines: Vec<String>,
    pub output_lines: Vec<String>,
    pub bottom_tab: BottomTab,
}

impl Default for UiModel {
    fn default() -> Self {
        Self {
            backend_choice: BackendChoice::Lldb,
            executable_path: String::new(),
            launch_args: String::new(),
            attach_pid: String::new(),
            status_line: "Idle".to_string(),
            stop_reason: StopReason::None,
            threads: Vec::new(),
            frames: Vec::new(),
            function_rows: Vec::new(),
            disassembly: Vec::new(),
            registers: None,
            register_diffs: Vec::new(),
            breakpoints: Vec::new(),
            trace: Vec::new(),
            memory_map_lines: Vec::new(),
            memory_base_input: "0x1000".to_string(),
            memory_bytes: Vec::new(),
            analysis_lines: Vec::new(),
            output_lines: Vec::new(),
            bottom_tab: BottomTab::Trace,
        }
    }
}

pub fn render(ctx: &egui::Context, model: &mut UiModel) -> Vec<UiEvent> {
    let mut events = Vec::new();

    handle_shortcuts(ctx, &mut events);
    top_bar(ctx, model, &mut events);
    left_session_panel(ctx, model, &mut events);
    right_registers_panel(ctx, model, &mut events);
    bottom_panel(ctx, model, &mut events);
    center_disassembly(ctx, model, &mut events);

    events
}

fn handle_shortcuts(ctx: &egui::Context, events: &mut Vec<UiEvent>) {
    ctx.input(|input| {
        if input.key_pressed(Key::F5) {
            events.push(UiEvent::Continue);
        }
        if input.key_pressed(Key::F10) {
            events.push(UiEvent::StepOver);
        }
        if input.key_pressed(Key::F11) {
            events.push(UiEvent::StepInto);
        }
        if input.modifiers.shift && input.key_pressed(Key::F11) {
            events.push(UiEvent::StepOut);
        }
        if input.key_pressed(Key::F6) {
            events.push(UiEvent::Pause);
        }
        if input.modifiers.command && input.key_pressed(Key::R) {
            events.push(UiEvent::Restart);
        }
    });
}

fn top_bar(ctx: &egui::Context, model: &mut UiModel, events: &mut Vec<UiEvent>) {
    egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
        egui::menu::bar(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Open Executable...").clicked() {
                    events.push(UiEvent::OpenExecutablePicker);
                    ui.close_menu();
                }
                if ui.button("Refresh").clicked() {
                    events.push(UiEvent::Refresh);
                    ui.close_menu();
                }
                if ui.button("Launch").clicked() {
                    events.push(UiEvent::Launch);
                    ui.close_menu();
                }
                if ui.button("Quit").clicked() {
                    events.push(UiEvent::Quit);
                    ui.close_menu();
                }
            });

            ui.menu_button("Edit", |ui| {
                if ui.button("Clear Trace").clicked() {
                    events.push(UiEvent::ClearTrace);
                    ui.close_menu();
                }
                if ui.button("Clear Output").clicked() {
                    events.push(UiEvent::ClearOutput);
                    ui.close_menu();
                }
            });

            ui.menu_button("Debug", |ui| {
                if ui.button("Launch").clicked() {
                    events.push(UiEvent::Launch);
                    ui.close_menu();
                }
                if ui.button("Continue (F5)").clicked() {
                    events.push(UiEvent::Continue);
                    ui.close_menu();
                }
                if ui.button("Pause (F6)").clicked() {
                    events.push(UiEvent::Pause);
                    ui.close_menu();
                }
                if ui.button("Step Into (F11)").clicked() {
                    events.push(UiEvent::StepInto);
                    ui.close_menu();
                }
                if ui.button("Step Over (F10)").clicked() {
                    events.push(UiEvent::StepOver);
                    ui.close_menu();
                }
                if ui.button("Step Out (Shift+F11)").clicked() {
                    events.push(UiEvent::StepOut);
                    ui.close_menu();
                }
                if ui.button("Stop").clicked() {
                    events.push(UiEvent::Stop);
                    ui.close_menu();
                }
            });

            ui.menu_button("View", |ui| {
                if ui.button("Stack Tab").clicked() {
                    events.push(UiEvent::SetBottomTab(BottomTab::Stack));
                    ui.close_menu();
                }
                if ui.button("Memory Tab").clicked() {
                    events.push(UiEvent::SetBottomTab(BottomTab::Memory));
                    ui.close_menu();
                }
                if ui.button("Trace Tab").clicked() {
                    events.push(UiEvent::SetBottomTab(BottomTab::Trace));
                    ui.close_menu();
                }
                if ui.button("Breakpoints Tab").clicked() {
                    events.push(UiEvent::SetBottomTab(BottomTab::Breakpoints));
                    ui.close_menu();
                }
                if ui.button("Output Tab").clicked() {
                    events.push(UiEvent::SetBottomTab(BottomTab::Output));
                    ui.close_menu();
                }
                if ui.button("Analysis Tab").clicked() {
                    events.push(UiEvent::SetBottomTab(BottomTab::Analysis));
                    ui.close_menu();
                }
            });

            ui.menu_button("Navigate", |ui| {
                if ui.button("Jump To Current Instruction").clicked() {
                    events.push(UiEvent::JumpToCurrentInstruction);
                    ui.close_menu();
                }
            });

            ui.menu_button("Analysis", |ui| {
                if ui.button("Refresh State").clicked() {
                    events.push(UiEvent::Refresh);
                    ui.close_menu();
                }
            });

            ui.menu_button("Help", |ui| {
                if ui.button("About tra86").clicked() {
                    events.push(UiEvent::ShowAbout);
                    ui.close_menu();
                }
            });

            ui.menu_button("Backend", |ui| {
                if ui
                    .selectable_label(model.backend_choice == BackendChoice::Mock, "Mock")
                    .clicked()
                {
                    events.push(UiEvent::SetBackend(BackendChoice::Mock));
                    ui.close_menu();
                }
                if ui
                    .selectable_label(model.backend_choice == BackendChoice::Lldb, "LLDB")
                    .clicked()
                {
                    events.push(UiEvent::SetBackend(BackendChoice::Lldb));
                    ui.close_menu();
                }
            });
        });

        ui.separator();
        ui.horizontal_wrapped(|ui| {
            ui.heading("tra86");
            ui.separator();

            ui.label("Executable:");
            ui.add(TextEdit::singleline(&mut model.executable_path).desired_width(280.0));
            if ui.button("Browse").clicked() {
                events.push(UiEvent::OpenExecutablePicker);
            }

            ui.label("Args:");
            ui.add(TextEdit::singleline(&mut model.launch_args).desired_width(200.0));
            if ui.button("Launch").clicked() {
                events.push(UiEvent::Launch);
            }

            ui.separator();
            ui.label("Attach PID:");
            ui.add(TextEdit::singleline(&mut model.attach_pid).desired_width(80.0));
            if ui.button("Attach").clicked() {
                events.push(UiEvent::Attach);
            }

            ui.separator();
            control_buttons(ui, events);
            if ui.button("Refresh").clicked() {
                events.push(UiEvent::Refresh);
            }
        });

        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("State: {}", model.status_line)).color(Color32::LIGHT_GREEN),
            );
            ui.separator();
            ui.label(format!("Stop reason: {:?}", model.stop_reason));
            ui.separator();
            ui.label("F5 continue | F10 step over | F11 step into | Shift+F11 step out | F6 pause");
        });
    });
}

fn control_buttons(ui: &mut egui::Ui, events: &mut Vec<UiEvent>) {
    if ui.button("Run/Continue").clicked() {
        events.push(UiEvent::Continue);
    }
    if ui.button("Pause").clicked() {
        events.push(UiEvent::Pause);
    }
    if ui.button("Step Into").clicked() {
        events.push(UiEvent::StepInto);
    }
    if ui.button("Step Over").clicked() {
        events.push(UiEvent::StepOver);
    }
    if ui.button("Step Out").clicked() {
        events.push(UiEvent::StepOut);
    }
    if ui.button("Restart").clicked() {
        events.push(UiEvent::Restart);
    }
    if ui.button("Stop").clicked() {
        events.push(UiEvent::Stop);
    }
}

fn left_session_panel(ctx: &egui::Context, model: &UiModel, events: &mut Vec<UiEvent>) {
    egui::SidePanel::left("threads_left")
        .default_width(260.0)
        .show(ctx, |ui| {
            let total_height = ui.available_height().max(300.0);
            let threads_h = (total_height * 0.26).max(100.0);
            let frames_h = (total_height * 0.26).max(100.0);
            let tree_h = (total_height - threads_h - frames_h - 16.0).max(120.0);

            ui.heading("Threads");
            ui.separator();
            ui.allocate_ui(egui::vec2(ui.available_width(), threads_h), |ui| {
                ScrollArea::vertical()
                    .id_salt("threads_list_scroll")
                    .show(ui, |ui| {
                        for thread in &model.threads {
                            let text = format!(
                                "#{} {} @ {}",
                                thread.id,
                                thread.name.as_deref().unwrap_or("unnamed"),
                                thread
                                    .instruction_pointer
                                    .map(|v| format!("0x{v:016x}"))
                                    .unwrap_or_else(|| "n/a".to_string())
                            );
                            let color = if thread.is_current {
                                Color32::LIGHT_BLUE
                            } else {
                                Color32::GRAY
                            };
                            ui.label(RichText::new(text).color(color));
                            ui.small(format!(
                                "status={:?} reason={:?}",
                                thread.status, thread.stop_reason
                            ));
                            ui.separator();
                        }
                        if model.threads.is_empty() {
                            ui.label("No threads");
                        }
                    });
            });

            ui.separator();
            ui.heading("Frames");
            ui.allocate_ui(egui::vec2(ui.available_width(), frames_h), |ui| {
                ScrollArea::vertical()
                    .id_salt("frames_list_scroll")
                    .show(ui, |ui| {
                        for frame in &model.frames {
                            let fn_name = frame
                                .function
                                .clone()
                                .unwrap_or_else(|| "unknown".to_string());
                            ui.label(
                                RichText::new(format!(
                                    "#{} 0x{:016x} {}",
                                    frame.index, frame.instruction_pointer, fn_name
                                ))
                                .monospace(),
                            );
                            if let Some(source) = &frame.source {
                                ui.small(format!("{}:{}", source.file, source.line));
                            }
                        }
                        if model.frames.is_empty() {
                            ui.label("No frames");
                        }
                    });
            });

            ui.separator();
            ui.heading("Program Tree");
            ui.allocate_ui(egui::vec2(ui.available_width(), tree_h), |ui| {
                ScrollArea::vertical()
                    .id_salt("program_tree_scroll")
                    .show(ui, |ui| {
                        if model.function_rows.is_empty() {
                            ui.label("No functions indexed");
                        } else {
                            for row in &model.function_rows {
                                let clicked = ui
                                    .selectable_label(false, RichText::new(row).monospace())
                                    .clicked();
                                if clicked {
                                    if let Some(address) = parse_address_from_row(row) {
                                        events.push(UiEvent::JumpDisassembly(address));
                                    }
                                }
                            }
                        }
                    });
            });
        });
}

fn right_registers_panel(ctx: &egui::Context, model: &UiModel, _events: &mut Vec<UiEvent>) {
    egui::SidePanel::right("registers_right")
        .default_width(330.0)
        .show(ctx, |ui| {
            ui.heading("Registers");
            ui.separator();

            let diffs: std::collections::BTreeMap<_, _> = model
                .register_diffs
                .iter()
                .map(|d| (d.name.as_str(), d))
                .collect();

            let total_height = ui.available_height().max(260.0);
            let registers_h = (total_height * 0.98).max(120.0);

            ui.allocate_ui(egui::vec2(ui.available_width(), registers_h), |ui| {
                ScrollArea::vertical()
                    .id_salt("registers_scroll")
                    .show(ui, |ui| {
                        if let Some(bank) = &model.registers {
                            for reg in &bank.registers {
                                let changed = diffs.contains_key(reg.name.as_str());
                                let text = format!("{:>8}  {}", reg.name, reg.value);
                                let color = if changed {
                                    Color32::YELLOW
                                } else {
                                    Color32::LIGHT_GRAY
                                };
                                ui.label(RichText::new(text).monospace().color(color));
                            }
                        } else {
                            ui.label("No register bank loaded");
                        }
                    });
            });
        });
}

fn center_disassembly(ctx: &egui::Context, model: &UiModel, events: &mut Vec<UiEvent>) {
    egui::CentralPanel::default().show(ctx, |ui| {
        ui.heading("Disassembly");
        ui.separator();

        ScrollArea::vertical()
            .id_salt("disassembly_scroll")
            .show(ui, |ui| {
                for line in &model.disassembly {
                    ui.horizontal(|ui| {
                        let bp_marker = if line.has_breakpoint { "●" } else { "○" };
                        if ui.button(bp_marker).clicked() {
                            events.push(UiEvent::ToggleBreakpoint(line.address));
                        }

                        if line.is_current {
                            ui.label(RichText::new("=>").color(Color32::LIGHT_GREEN));
                        } else {
                            ui.label("  ");
                        }

                        ui.label(
                            RichText::new(format!("0x{:016x}", line.address))
                                .monospace()
                                .color(Color32::LIGHT_BLUE),
                        );
                        ui.label(
                            RichText::new(bytes_to_hex(&line.bytes))
                                .monospace()
                                .color(Color32::GRAY),
                        );
                        ui.label(RichText::new(&line.mnemonic).monospace().strong());
                        ui.label(RichText::new(&line.operands).monospace());

                        if let Some(target) = line.branch_target {
                            if ui
                                .small_button(format!("-> 0x{target:x}"))
                                .on_hover_text("Jump to target in memory view")
                                .clicked()
                            {
                                events.push(UiEvent::JumpMemory(target));
                            }
                        }

                        if let Some(source) = &line.source {
                            ui.label(
                                RichText::new(format!("{}:{}", source.file, source.line))
                                    .color(Color32::GRAY),
                            );
                        }
                    });
                }
            });
    });
}

fn bottom_panel(ctx: &egui::Context, model: &mut UiModel, events: &mut Vec<UiEvent>) {
    egui::TopBottomPanel::bottom("bottom_tabs")
        .resizable(true)
        .default_height(260.0)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                tab_button(ui, model, BottomTab::Stack, "Stack");
                tab_button(ui, model, BottomTab::Memory, "Memory");
                tab_button(ui, model, BottomTab::Breakpoints, "Breakpoints");
                tab_button(ui, model, BottomTab::Trace, "Trace");
                tab_button(ui, model, BottomTab::Analysis, "Analysis");
                tab_button(ui, model, BottomTab::Output, "Output");
            });
            ui.separator();

            match model.bottom_tab {
                BottomTab::Stack => render_stack(ui, model, events),
                BottomTab::Memory => render_memory(ui, model, events),
                BottomTab::Breakpoints => render_breakpoints(ui, model, events),
                BottomTab::Trace => render_trace(ui, model),
                BottomTab::Analysis => render_analysis(ui, model),
                BottomTab::Output => render_output(ui, model),
            }
        });
}

fn tab_button(ui: &mut egui::Ui, model: &mut UiModel, tab: BottomTab, title: &str) {
    if ui
        .selectable_label(model.bottom_tab == tab, title)
        .clicked()
    {
        model.bottom_tab = tab;
    }
}

fn render_stack(ui: &mut egui::Ui, model: &UiModel, events: &mut Vec<UiEvent>) {
    ui.heading("Stack Visualizer");
    let sp_val = model
        .registers
        .as_ref()
        .and_then(|bank| parse_register_addr(bank, "rsp"));
    let bp_val = model
        .registers
        .as_ref()
        .and_then(|bank| parse_register_addr(bank, "rbp"));

    ui.horizontal(|ui| {
        ui.label(format!(
            "SP: {}",
            sp_val
                .map(|v| format!("0x{v:016x}"))
                .unwrap_or_else(|| "n/a".to_string())
        ));
        ui.label(format!(
            "BP: {}",
            bp_val
                .map(|v| format!("0x{v:016x}"))
                .unwrap_or_else(|| "n/a".to_string())
        ));
    });

    ui.separator();
    ScrollArea::vertical()
        .id_salt("stack_scroll")
        .show(ui, |ui| {
            if model.memory_bytes.is_empty() {
                ui.label("No memory snapshot loaded");
                return;
            }

            for (idx, chunk) in model.memory_bytes.chunks(16).enumerate() {
                let addr = parse_address(&model.memory_base_input).unwrap_or(0) + (idx as u64 * 16);
                let mut text = RichText::new(format!(
                    "0x{addr:016x}: {:<48}  {}",
                    bytes_to_hex(chunk),
                    bytes_to_ascii(chunk)
                ))
                .monospace();

                if Some(addr) == sp_val {
                    text = text.color(Color32::LIGHT_GREEN);
                } else if Some(addr) == bp_val {
                    text = text.color(Color32::LIGHT_BLUE);
                }

                if ui.selectable_label(false, text).clicked() {
                    events.push(UiEvent::JumpMemory(addr));
                }
            }
        });
}

fn render_memory(ui: &mut egui::Ui, model: &mut UiModel, events: &mut Vec<UiEvent>) {
    ui.horizontal(|ui| {
        ui.label("Address:");
        ui.add(TextEdit::singleline(&mut model.memory_base_input).desired_width(140.0));
        if ui.button("Jump").clicked() {
            if let Some(addr) = parse_address(&model.memory_base_input) {
                events.push(UiEvent::JumpMemory(addr));
            }
        }
    });

    ui.separator();
    ui.label("Memory map:");
    ScrollArea::vertical()
        .id_salt("memory_regions_scroll")
        .max_height(64.0)
        .show(ui, |ui| {
            for line in &model.memory_map_lines {
                ui.label(RichText::new(line).monospace().color(Color32::GRAY));
            }
        });

    ui.separator();
    ScrollArea::vertical()
        .id_salt("memory_hex_scroll")
        .show(ui, |ui| {
            for (idx, chunk) in model.memory_bytes.chunks(16).enumerate() {
                let addr = parse_address(&model.memory_base_input).unwrap_or(0) + (idx as u64 * 16);
                ui.label(
                    RichText::new(format!(
                        "0x{addr:016x}: {:<48}  {}",
                        bytes_to_hex(chunk),
                        bytes_to_ascii(chunk)
                    ))
                    .monospace(),
                );
            }
        });

    if model.memory_bytes.len() >= 8 {
        ui.separator();
        let v = &model.memory_bytes;
        let u16v = u16::from_le_bytes([v[0], v[1]]);
        let u32v = u32::from_le_bytes([v[0], v[1], v[2], v[3]]);
        let u64v = u64::from_le_bytes([v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7]]);
        let i32v = i32::from_le_bytes([v[0], v[1], v[2], v[3]]);
        let i64v = i64::from_le_bytes([v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7]]);
        let f32v = f32::from_le_bytes([v[0], v[1], v[2], v[3]]);
        let f64v = f64::from_le_bytes([v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7]]);
        ui.label(format!(
            "u16={} u32={} u64={} i32={} i64={} f32={:.4} f64={:.4} ptr=0x{u64v:016x}",
            u16v, u32v, u64v, i32v, i64v, f32v, f64v
        ));
    }
}

fn render_trace(ui: &mut egui::Ui, model: &UiModel) {
    ui.heading("Instruction History");
    ScrollArea::vertical()
        .id_salt("trace_scroll")
        .show(ui, |ui| {
            for entry in model.trace.iter().rev().take(500) {
                render_trace_row(ui, entry);
            }
            if model.trace.is_empty() {
                ui.label("No trace events yet. Step to start collecting history.");
            }
        });
}

fn render_breakpoints(ui: &mut egui::Ui, model: &UiModel, events: &mut Vec<UiEvent>) {
    ui.heading("Breakpoints");
    ScrollArea::vertical()
        .id_salt("breakpoints_bottom_scroll")
        .show(ui, |ui| {
            for bp in &model.breakpoints {
                ui.horizontal(|ui| {
                    ui.label(format!("#{}", bp.id));
                    ui.label(match &bp.location {
                        BreakpointLocation::Address(addr) => format!("0x{addr:016x}"),
                        BreakpointLocation::Symbol(sym) => sym.clone(),
                    });
                    ui.small(format!("hits {}", bp.hit_count));
                    if ui.button("remove").clicked() {
                        events.push(UiEvent::RemoveBreakpoint(bp.id));
                    }
                });
            }
            if model.breakpoints.is_empty() {
                ui.label("No breakpoints");
            }
        });
}

fn render_trace_row(ui: &mut egui::Ui, entry: &TraceEvent) {
    let ts = local_timestamp(entry.timestamp);
    let mut extra = String::new();
    if let Some(delta) = &entry.delta {
        extra = format!(
            " | regs:{} sp:{:+} cf:{:?} hints:{:?}",
            delta.changed_registers.len(),
            delta.stack_pointer_delta,
            delta.control_flow,
            delta.hints
        );
    }

    let row = format!(
        "{} T{} 0x{:016x} {:<8} {:<24} stop={:?}{}",
        ts,
        entry.thread_id,
        entry.instruction.address,
        entry.instruction.mnemonic,
        entry.instruction.operands,
        entry.stop_reason,
        extra
    );
    ui.label(RichText::new(row).monospace());
}

fn render_analysis(ui: &mut egui::Ui, model: &UiModel) {
    ui.heading("Analysis");
    ScrollArea::vertical()
        .id_salt("analysis_scroll")
        .show(ui, |ui| {
            if model.analysis_lines.is_empty() {
                ui.label("No analysis yet. Load target and step execution.");
                return;
            }
            for line in &model.analysis_lines {
                ui.label(RichText::new(line).monospace());
            }
        });
}

fn render_output(ui: &mut egui::Ui, model: &UiModel) {
    ScrollArea::vertical()
        .id_salt("output_scroll")
        .show(ui, |ui| {
            for line in &model.output_lines {
                ui.label(RichText::new(line).monospace().color(Color32::LIGHT_GRAY));
            }
        });
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn bytes_to_ascii(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| {
            if b.is_ascii_graphic() {
                *b as char
            } else {
                '.'
            }
        })
        .collect()
}

fn parse_address(input: &str) -> Option<Address> {
    let trimmed = input.trim();
    if let Some(hex) = trimmed.strip_prefix("0x") {
        u64::from_str_radix(hex, 16).ok()
    } else {
        trimmed.parse::<u64>().ok()
    }
}

fn parse_address_from_row(row: &str) -> Option<Address> {
    row.split_whitespace()
        .find(|part| part.starts_with("0x"))
        .and_then(parse_address)
}

fn parse_register_addr(bank: &RegisterBank, name: &str) -> Option<Address> {
    bank.registers
        .iter()
        .find(|reg| reg.name == name)
        .and_then(|reg| parse_address(&reg.value))
}

fn local_timestamp(ts: DateTime<chrono::Utc>) -> String {
    let local = DateTime::<Local>::from(ts);
    local.format("%H:%M:%S%.3f").to_string()
}
