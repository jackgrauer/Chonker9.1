use eframe::egui;
use std::{process::Command, sync::{Arc, Mutex}};

#[derive(Debug, Clone)]
struct SpatialElement {
    content: String,
    hpos: f32,
    vpos: f32,
    width: f32,
    height: f32,
}

#[derive(Debug, Clone)]
struct TerminalMetrics {
    cell_width_pts: f32,
    cell_height_pts: f32,
}

impl TerminalMetrics {
    fn new() -> Self {
        Self {
            cell_width_pts: 8.0,  // Standard monospace width in points
            cell_height_pts: 15.0, // Standard line height in points
        }
    }
    
    fn pdf_to_terminal(&self, pdf_x: f32, pdf_y: f32) -> (u16, u16) {
        let col = (pdf_x / self.cell_width_pts) as u16;
        let row = (pdf_y / self.cell_height_pts) as u16;
        (col, row)
    }
}

struct ChonkerApp {
    pdf_path: String,
    raw_xml: String,
    spatial_elements: Vec<SpatialElement>,
    terminal_metrics: TerminalMetrics,
    show_xml_debug: bool,
    xml_scroll: usize,
    terminal_output: Arc<Mutex<String>>,
    // Text editing capabilities
    rope: ropey::Rope,
    cursor_pos: usize,
    selection_start: Option<usize>,
    selection_end: Option<usize>,
    modified: bool,
}

impl Default for ChonkerApp {
    fn default() -> Self {
        Self {
            pdf_path: "/Users/jack/Documents/chonker_test.pdf".to_string(),
            raw_xml: String::new(),
            spatial_elements: Vec::new(),
            terminal_metrics: TerminalMetrics::new(),
            show_xml_debug: false,
            xml_scroll: 0,
            terminal_output: Arc::new(Mutex::new(String::new())),
            rope: ropey::Rope::new(),
            cursor_pos: 0,
            selection_start: None,
            selection_end: None,
            modified: false,
        }
    }
}

impl ChonkerApp {
    fn load_pdf(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // Extract PDF using pdfalto
        let output = Command::new("pdfalto")
            .args([
                "-f", "1", "-l", "1",   // Just page 1 for now
                "-readingOrder",        // Follow visual reading order
                "-noImage",            // Skip image extraction for speed
                "-noLineNumbers",      // Clean output without line numbers
                &self.pdf_path,
                "/dev/stdout"
            ])
            .output()?;
        
        if !output.status.success() {
            return Err("pdfalto failed".into());
        }
        
        self.raw_xml = String::from_utf8_lossy(&output.stdout).to_string();
        self.parse_spatial_elements()?;
        self.build_rope_from_elements();
        
        Ok(())
    }
    
    fn parse_spatial_elements(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        use quick_xml::{Reader, events::Event};
        
        let mut reader = Reader::from_str(&self.raw_xml);
        let mut buf = Vec::new();
        self.spatial_elements.clear();
        
        let mut in_page = false;
        
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                    let tag_bytes = e.name().as_ref().to_vec();
                    let tag_name = String::from_utf8_lossy(&tag_bytes);
                    
                    if tag_name == "Page" {
                        in_page = true;
                    } else if tag_name == "String" && in_page {
                        let mut content = String::new();
                        let mut hpos = 0.0;
                        let mut vpos = 0.0;
                        let mut width = 0.0;
                        let mut height = 0.0;
                        
                        for attr in e.attributes() {
                            if let Ok(attr) = attr {
                                let key = String::from_utf8_lossy(attr.key.as_ref());
                                let value = String::from_utf8_lossy(&attr.value);
                                
                                match key.as_ref() {
                                    "CONTENT" => content = value.to_string(),
                                    "HPOS" => hpos = value.parse().unwrap_or(0.0),
                                    "VPOS" => vpos = value.parse().unwrap_or(0.0),
                                    "WIDTH" => width = value.parse().unwrap_or(0.0),
                                    "HEIGHT" => height = value.parse().unwrap_or(0.0),
                                    _ => {}
                                }
                            }
                        }
                        
                        if !content.is_empty() {
                            self.spatial_elements.push(SpatialElement {
                                content,
                                hpos,
                                vpos,
                                width,
                                height,
                            });
                        }
                    }
                }
                Ok(Event::End(e)) => {
                    let tag_bytes = e.name().as_ref().to_vec();
                    let tag_name = String::from_utf8_lossy(&tag_bytes);
                    
                    if tag_name == "Page" {
                        in_page = false;
                    }
                }
                Ok(Event::Eof) => break,
                _ => {}
            }
            buf.clear();
        }
        
        Ok(())
    }
    
    fn generate_readable_text(&self) -> String {
        // Group elements into lines and create readable text with proper spacing
        let mut lines: Vec<Vec<&SpatialElement>> = Vec::new();
        
        // Sort elements by vertical position first
        let mut sorted_elements: Vec<&SpatialElement> = self.spatial_elements.iter().collect();
        sorted_elements.sort_by(|a, b| a.vpos.partial_cmp(&b.vpos).unwrap());
        
        // Group into lines (within 8 pixels vertically)
        for element in sorted_elements {
            let found_line = lines.iter_mut().find(|line| {
                if let Some(first) = line.first() {
                    (element.vpos - first.vpos).abs() < 8.0
                } else {
                    false
                }
            });
            
            if let Some(line) = found_line {
                line.push(element);
            } else {
                lines.push(vec![element]);
            }
        }
        
        // Sort words within each line by horizontal position
        for line in &mut lines {
            line.sort_by(|a, b| a.hpos.partial_cmp(&b.hpos).unwrap());
        }
        
        // Reconstruct readable text
        let mut output = String::new();
        for line in lines {
            let mut line_text = String::new();
            let mut last_end_pos = 0.0;
            
            for element in line {
                if !line_text.is_empty() {
                    // Calculate gap between words
                    let gap = element.hpos - last_end_pos;
                    if gap > 3.0 {  // Significant gap - add spaces
                        let spaces = ((gap / 8.0) as usize).min(10).max(1);  // Based on typical char width
                        line_text.push_str(&" ".repeat(spaces));
                    } else {
                        line_text.push(' '); // Normal single space
                    }
                }
                
                line_text.push_str(&element.content);
                last_end_pos = element.hpos + element.width;
            }
            
            output.push_str(&line_text);
            output.push('\n');
        }
        
        output
    }
    
    fn build_rope_from_elements(&mut self) {
        // Build rope text buffer from spatial elements
        let readable_text = self.generate_readable_text();
        self.rope = ropey::Rope::from_str(&readable_text);
        self.cursor_pos = 0;
        self.modified = false;
    }
    
    fn render_readable_text(&self, ui: &mut egui::Ui) {
        // The simple approach that gave good paragraph readability
        let readable_text = self.generate_readable_text();
        
        ui.add(egui::TextEdit::multiline(&mut readable_text.as_str())
            .font(egui::TextStyle::Monospace)
            .desired_width(f32::INFINITY)
            .desired_rows(30));
    }
    
    fn format_xml(&self) -> String {
        // Simple XML formatting for better readability
        let mut formatted = String::new();
        let mut indent_level: usize = 0;
        let lines: Vec<&str> = self.raw_xml.lines().collect();
        
        for line in lines {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            
            // Decrease indent for closing tags
            if trimmed.starts_with("</") {
                indent_level = indent_level.saturating_sub(1);
            }
            
            // Add indentation
            formatted.push_str(&"  ".repeat(indent_level));
            formatted.push_str(trimmed);
            formatted.push('\n');
            
            // Increase indent for opening tags (but not self-closing)
            if trimmed.starts_with('<') && !trimmed.starts_with("</") && !trimmed.ends_with("/>") && !trimmed.starts_with("<?") {
                indent_level += 1;
            }
        }
        
        formatted
    }
}

impl eframe::App for ChonkerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Top panel with controls
        egui::TopBottomPanel::top("controls").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("📁 Load PDF").clicked() {
                    if let Err(e) = self.load_pdf() {
                        eprintln!("Error loading PDF: {}", e);
                    }
                }
                
                ui.separator();
                
                if ui.button("🔍 XML Debug").clicked() {
                    self.show_xml_debug = !self.show_xml_debug;
                }
                
                if self.show_xml_debug {
                    ui.label("📋 Debug Mode");
                    if ui.button("💾 Save XML").clicked() {
                        if let Err(e) = std::fs::write("chonker9_debug.xml", &self.raw_xml) {
                            eprintln!("Error saving XML: {}", e);
                        }
                    }
                } else {
                    if ui.button("💾 Save Text").clicked() {
                        let content = self.rope.to_string();
                        if let Err(e) = std::fs::write("chonker9_edited.txt", content) {
                            eprintln!("Error saving text: {}", e);
                        }
                    }
                }
            });
        });
        
        // Main content area
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.show_xml_debug {
                // XML Debug View - Formatted and Readable
                ui.heading("🔍 Raw ALTO XML Structure");
                
                // Format XML for better readability
                let formatted_xml = self.format_xml();
                
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add(egui::TextEdit::multiline(&mut formatted_xml.as_str())
                        .font(egui::TextStyle::Monospace)
                        .code_editor()
                        .desired_width(f32::INFINITY)
                        .desired_rows(40));
                });
            } else {
                // PDF View with Absolute Coordinates
                ui.horizontal(|ui| {
                    ui.heading("📄 PDF Content (Absolute Positioning)");
                    ui.separator();
                    if ui.button("📝 Readable Text").clicked() {
                        // Toggle between absolute and readable view
                    }
                    if self.modified {
                        ui.label("*MODIFIED*");
                    }
                });
                
                egui::ScrollArea::both()
                    .auto_shrink([false, false])  // Allow unlimited scrolling
                    .show(ui, |ui| {
                        if !self.spatial_elements.is_empty() {
                            // Back to basic readable text approach
                            self.render_readable_text(ui);
                        } else {
                            ui.label("Click '📁 Load PDF' to display content");
                        }
                    });
            }
        });
    }
}

fn main() -> Result<(), eframe::Error> {
    println!("🚀 Starting Chonker9...");
    
    let mut app = ChonkerApp::default();
    
    // Auto-load the default PDF
    println!("📁 Loading PDF...");
    if let Err(e) = app.load_pdf() {
        eprintln!("Error auto-loading PDF: {}", e);
    } else {
        println!("✅ PDF loaded successfully - {} elements", app.spatial_elements.len());
    }
    
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 700.0])
            .with_title("Chonker9 - PDF Editor"),
        ..Default::default()
    };
    
    println!("🖥️ Creating window...");
    eframe::run_native(
        "Chonker9",
        options,
        Box::new(|_cc| {
            println!("✅ Window created");
            Ok(Box::new(app))
        }),
    )
}