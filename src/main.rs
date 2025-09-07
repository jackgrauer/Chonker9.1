use eframe::egui;
use std::{process::Command, sync::{Arc, Mutex}, thread, time::Duration};
use cosmic_text::{FontSystem, Buffer, Metrics, Attrs, Shaping, Family, SwashCache};
use rfd::FileDialog;

mod spatial_text;
use spatial_text::{SpatialTextBuffer, SpatialCursor, ElementRange};

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
    // Click-to-edit state
    editing_element: Option<usize>,  // Which element is being edited
    edit_text: String,               // Current edit text
    // WYSIWYG spatial editing system
    spatial_buffer: SpatialTextBuffer,
    spatial_cursor: SpatialCursor,
    // Cosmic-text for professional typography  
    font_system: FontSystem,
    text_buffer: Buffer,
    swash_cache: SwashCache,
    wysiwyg_mode: bool,              // Toggle between old and new system
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
            editing_element: None,
            edit_text: String::new(),
            spatial_buffer: SpatialTextBuffer::new(),
            spatial_cursor: SpatialCursor::new(),
            // Initialize cosmic-text for Kitty-quality typography
            font_system: FontSystem::new(),
            text_buffer: {
                let mut fs = FontSystem::new();
                let mut buffer = Buffer::new(&mut fs, Metrics::new(14.0, 18.0));
                
                // Set Kitty-like font attributes for superior rendering
                let kitty_attrs = Attrs::new()
                    .family(Family::Name("SF Mono"))  // Kitty's preferred font on macOS
                    .weight(cosmic_text::Weight::NORMAL)
                    .style(cosmic_text::Style::Normal);
                
                buffer.set_text(&mut fs, "Initial text", kitty_attrs, Shaping::Advanced);
                buffer
            },
            swash_cache: SwashCache::new(),
            wysiwyg_mode: false,
        }
    }
}

impl ChonkerApp {
    fn load_pdf(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // Check if PDF file exists
        if !std::path::Path::new(&self.pdf_path).exists() {
            return Err(format!("PDF file not found: {}", self.pdf_path).into());
        }
        
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
        
        // Initialize WYSIWYG spatial buffer
        let elements_for_spatial: Vec<(String, f32, f32, f32, f32)> = self.spatial_elements.iter()
            .map(|e| (e.content.clone(), e.hpos, e.vpos, e.width, e.height))
            .collect();
        self.spatial_buffer = SpatialTextBuffer::from_alto_elements(&elements_for_spatial);
        
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
        
        // Reconstruct readable text with section spacing
        let mut output = String::new();
        let mut last_vpos = 0.0;
        
        for line in lines {
            if !line.is_empty() {
                let current_vpos = line[0].vpos;
                
                // Add extra spacing for large vertical gaps (section breaks)
                if last_vpos > 0.0 {
                    let vertical_gap = current_vpos - last_vpos;
                    if vertical_gap > 15.0 {  // Large gap - add extra line breaks
                        let extra_lines = ((vertical_gap / 12.0) as usize).min(3).max(1);
                        output.push_str(&"\n".repeat(extra_lines));
                    }
                }
                
                let mut line_text = String::new();
                let mut last_end_pos = 0.0;
                
                for element in line {
                    if !line_text.is_empty() {
                        // Better spacing calculation for good kerning
                        let gap = element.hpos - last_end_pos;
                        if gap > 6.0 {  // Large gap - multiple spaces
                            let spaces = ((gap / 6.0) as usize).min(8).max(2);
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
                last_vpos = current_vpos;
            }
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
    
    fn render_hybrid_smart(&mut self, ui: &mut egui::Ui) {
        let canvas_width = 3000.0;
        let canvas_height = 2000.0;
        
        let (response, painter) = ui.allocate_painter(
            egui::Vec2::new(canvas_width, canvas_height), 
            egui::Sense::click_and_drag()
        );
        
        // ALTO coordinates are in points (1/72 inch), need to scale for pixel display
        let scale_x = 1.2;  // Slightly expand horizontal for readability
        let scale_y = 1.0;  // Keep vertical as-is
        
        // Detect table elements (numbers, currency, short content in columns)
        let mut table_elements = Vec::new();
        let mut paragraph_elements = Vec::new();
        
        for element in &self.spatial_elements {
            let content = element.content.trim();
            
            // More precise table detection: actual table region VPOS 409-517
            let is_in_table_region = element.vpos >= 409.0 && element.vpos <= 517.0;
            let is_table_content = content.contains('$') ||           // Currency values
                                  content == "N/A" ||                // Table placeholders  
                                  content.contains('%') ||           // Percentages
                                  (content.chars().all(|c| c.is_numeric()) && content.len() == 4); // Years like 2011, 2012
            
            if is_in_table_region && is_table_content {
                table_elements.push(element);
            } else {
                paragraph_elements.push(element);
            }
        }
        
        // Render table elements with exact positioning (good for tables)
        for element in table_elements {
            let pos = egui::Pos2::new(
                element.hpos * scale_x,
                element.vpos * scale_y
            );
            
            painter.text(
                pos,
                egui::Align2::LEFT_TOP,
                &element.content,
                egui::FontId::monospace(12.0),
                egui::Color32::from_rgb(150, 255, 150) // Green for tables
            );
        }
        
        // Render paragraph elements with automatic spacing to prevent jumbling
        for element in paragraph_elements {
            let pos = egui::Pos2::new(
                element.hpos * scale_x,
                element.vpos * scale_y
            );
            
            // Add a space after each word to prevent jumbling
            let spaced_content = format!("{} ", element.content);
            
            painter.text(
                pos,
                egui::Align2::LEFT_TOP,
                &spaced_content,
                egui::FontId::monospace(12.0),
                egui::Color32::WHITE
            );
        }
        
        // Handle clicks for editing
        if response.clicked() {
            if let Some(click_pos) = response.interact_pointer_pos() {
                // Find which element was clicked
                let clicked_element = self.find_element_at_position(click_pos, scale_x, scale_y);
                if let Some(elem_idx) = clicked_element {
                    // Start editing this element
                    self.editing_element = Some(elem_idx);
                    self.edit_text = self.spatial_elements[elem_idx].content.clone();
                    self.modified = true;
                }
            }
        }
    }
    
    fn generate_readable_text_from_elements(&self, elements: &[&SpatialElement]) -> String {
        // Same line reconstruction logic but for subset of elements
        let mut lines: Vec<Vec<&SpatialElement>> = Vec::new();
        let mut sorted_elements: Vec<&SpatialElement> = elements.iter().cloned().collect();
        sorted_elements.sort_by(|a, b| a.vpos.partial_cmp(&b.vpos).unwrap());
        
        // Group into lines
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
        
        // Sort within lines and reconstruct
        for line in &mut lines {
            line.sort_by(|a, b| a.hpos.partial_cmp(&b.hpos).unwrap());
        }
        
        let mut output = String::new();
        let mut last_vpos = 0.0;
        
        for line in lines {
            if !line.is_empty() {
                let current_vpos = line[0].vpos;
                
                // Add section spacing
                if last_vpos > 0.0 {
                    let vertical_gap = current_vpos - last_vpos;
                    if vertical_gap > 15.0 {
                        let extra_lines = ((vertical_gap / 12.0) as usize).min(3).max(1);
                        output.push_str(&"\n".repeat(extra_lines));
                    }
                }
                
                let mut line_text = String::new();
                let mut last_end_pos = 0.0;
                
                for element in line {
                    if !line_text.is_empty() {
                        let gap = element.hpos - last_end_pos;
                        if gap > 3.0 {
                            let spaces = ((gap / 8.0) as usize).min(10).max(1);
                            line_text.push_str(&" ".repeat(spaces));
                        } else {
                            line_text.push(' ');
                        }
                    }
                    
                    line_text.push_str(&element.content);
                    last_end_pos = element.hpos + element.width;
                }
                
                output.push_str(&line_text);
                output.push('\n');
                last_vpos = current_vpos;
            }
        }
        
        output
    }
    
    fn render_paragraphs_with_positioning(&self, elements: &[&SpatialElement], painter: &egui::Painter, scale_x: f32, scale_y: f32) {
        // Group elements into lines but preserve horizontal positioning
        let mut lines: Vec<Vec<&SpatialElement>> = Vec::new();
        let mut sorted_elements: Vec<&SpatialElement> = elements.iter().cloned().collect();
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
        
        // Render each line at its proper position with spacing
        for line in lines {
            if line.is_empty() { continue; }
            
            let mut sorted_line = line.clone();
            sorted_line.sort_by(|a, b| a.hpos.partial_cmp(&b.hpos).unwrap());
            
            // Use the leftmost element's position as the line start
            let line_y = sorted_line[0].vpos * scale_y;
            let line_x = sorted_line[0].hpos * scale_x;  // Start at actual left margin
            
            // Build line text with proper spacing
            let mut line_text = String::new();
            let mut last_end_pos = 0.0;
            
            for element in sorted_line {
                if !line_text.is_empty() {
                    let gap = element.hpos - last_end_pos;
                    if gap > 3.0 {
                        let spaces = ((gap / 8.0) as usize).min(10).max(1);
                        line_text.push_str(&" ".repeat(spaces));
                    } else {
                        line_text.push(' ');
                    }
                }
                
                line_text.push_str(&element.content);
                last_end_pos = element.hpos + element.width;
            }
            
            // Render the line at its proper horizontal position
            painter.text(
                egui::Pos2::new(line_x, line_y),
                egui::Align2::LEFT_TOP,
                &line_text,
                egui::FontId::monospace(12.0),
                egui::Color32::WHITE
            );
        }
    }
    
    fn render_spaced_elements(&self, elements: &[&SpatialElement], painter: &egui::Painter, scale_x: f32, scale_y: f32, color: egui::Color32) {
        // Group elements by line (same VPOS) to add proper spacing
        let mut lines: std::collections::HashMap<i32, Vec<&SpatialElement>> = std::collections::HashMap::new();
        
        // Group elements by vertical position (rounded to handle minor variations)
        for element in elements {
            let vpos_key = (element.vpos * scale_y) as i32;
            lines.entry(vpos_key).or_insert_with(Vec::new).push(element);
        }
        
        // Render each line with proper spacing
        for (_vpos, mut line_elements) in lines {
            // Sort by horizontal position
            line_elements.sort_by(|a, b| a.hpos.partial_cmp(&b.hpos).unwrap());
            
            // Render each element with spacing consideration
            for (i, element) in line_elements.iter().enumerate() {
                let mut display_content = element.content.clone();
                
                // Add space after element if there's a significant gap to the next element
                if i < line_elements.len() - 1 {
                    let next_element = line_elements[i + 1];
                    let gap = next_element.hpos - (element.hpos + element.width);
                    
                    // If there's a significant gap (>3 pixels), add spaces
                    if gap > 3.0 {
                        let spaces_needed = ((gap / 8.0) as usize).min(10).max(1);
                        display_content.push_str(&" ".repeat(spaces_needed));
                    } else {
                        display_content.push(' '); // Single space for normal word separation
                    }
                }
                
                // Render at exact ALTO position
                let pos = egui::Pos2::new(
                    element.hpos * scale_x,
                    element.vpos * scale_y
                );
                
                painter.text(
                    pos,
                    egui::Align2::LEFT_TOP,
                    &display_content,
                    egui::FontId::monospace(12.0),
                    color
                );
            }
        }
    }
    
    fn find_element_at_position(&self, click_pos: egui::Pos2, scale_x: f32, scale_y: f32) -> Option<usize> {
        // Find the closest element to the click position
        let mut closest_distance = f32::MAX;
        let mut closest_element = None;
        
        for (i, element) in self.spatial_elements.iter().enumerate() {
            // Calculate element's screen position
            let element_pos = egui::Pos2::new(
                element.hpos * scale_x,
                element.vpos * scale_y
            );
            let element_size = egui::Vec2::new(element.width * scale_x, 20.0); // Assume ~20px height
            let element_rect = egui::Rect::from_min_size(element_pos, element_size);
            
            // Check if click is within element bounds
            if element_rect.contains(click_pos) {
                return Some(i);
            }
            
            // Otherwise, find closest element
            let distance = click_pos.distance(element_pos);
            if distance < closest_distance {
                closest_distance = distance;
                closest_element = Some(i);
            }
        }
        
        // Return closest element if click was reasonably close (within 50 pixels)
        if closest_distance < 50.0 {
            closest_element
        } else {
            None
        }
    }
    
    fn render_wysiwyg_mode(&mut self, ui: &mut egui::Ui) {
        let canvas_width = 3000.0;
        let canvas_height = 2000.0;
        
        let (response, painter) = ui.allocate_painter(
            egui::Vec2::new(canvas_width, canvas_height), 
            egui::Sense::click_and_drag()
        );
        
        // Handle clicks for cursor positioning
        if response.clicked() {
            if let Some(click_pos) = response.interact_pointer_pos() {
                self.spatial_cursor.move_to_screen_position(click_pos, &self.spatial_buffer);
            }
        }
        
        // Render each element using current rope content at exact ALTO positions
        for (_i, element_range) in self.spatial_buffer.element_ranges.iter().enumerate() {
            // Get current text from rope (this is the key - live text, not original)
            let current_text = if element_range.rope_start < self.spatial_buffer.rope.len_chars() {
                self.spatial_buffer.rope.slice(element_range.rope_start..element_range.rope_end.min(self.spatial_buffer.rope.len_chars())).to_string()
            } else {
                String::new()
            };
            
            // Render at exact ALTO coordinates (no zoom/pan for now - keep it simple)
            let pos = egui::Pos2::new(
                element_range.visual_bounds.min.x,
                element_range.visual_bounds.min.y
            );
            
            // Render text at spatial position
            if !current_text.is_empty() {
                painter.text(
                    pos,
                    egui::Align2::LEFT_TOP,
                    &current_text,
                    egui::FontId::monospace(12.0),
                    if element_range.modified { 
                        egui::Color32::from_rgb(255, 200, 100) // Orange for modified
                    } else { 
                        egui::Color32::WHITE 
                    }
                );
            }
            
            // Show bounds if element is overflowing
            if element_range.overflow {
                let bounds_rect = egui::Rect::from_min_size(pos, 
                    egui::Vec2::new(element_range.visual_bounds.width(), 15.0));
                painter.rect_stroke(bounds_rect, 0.0, egui::Stroke::new(1.0, egui::Color32::RED));
            }
        }
        
        // Update and render cursor
        self.spatial_cursor.update_position(&self.spatial_buffer);
        self.spatial_cursor.render(&painter);
        
        // Handle keyboard input for text editing
        ui.input(|i| {
            // Handle text input
            for event in &i.events {
                match event {
                    egui::Event::Text(text) => {
                        // Insert text at current cursor position
                        self.spatial_buffer.insert_text(self.spatial_cursor.rope_pos, text);
                        self.spatial_cursor.rope_pos += text.chars().count();
                        self.modified = true;
                    }
                    egui::Event::Key { key, pressed: true, .. } => {
                        match key {
                            egui::Key::Backspace => {
                                if self.spatial_cursor.rope_pos > 0 {
                                    self.spatial_buffer.delete_range(self.spatial_cursor.rope_pos - 1, self.spatial_cursor.rope_pos);
                                    self.spatial_cursor.rope_pos -= 1;
                                    self.modified = true;
                                    // XML updates automatically
                                }
                            }
                            egui::Key::ArrowLeft => {
                                if self.spatial_cursor.rope_pos > 0 {
                                    self.spatial_cursor.rope_pos -= 1;
                                }
                            }
                            egui::Key::ArrowRight => {
                                if self.spatial_cursor.rope_pos < self.spatial_buffer.rope.len_chars() {
                                    self.spatial_cursor.rope_pos += 1;
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
        });
    }
    
    fn render_wysiwyg_readable(&mut self, ui: &mut egui::Ui) {
        // Use ENTIRE right quadrant - allocate ALL available space
        let full_size = ui.available_size();
        
        let (response, painter) = ui.allocate_painter(
            full_size, // Use complete available area
            egui::Sense::click_and_drag()
        );
        
        let scale_x = 1.2;
        let scale_y = 1.0;
        
        // Use the readable paragraph rendering approach
        let mut table_elements = Vec::new();
        let mut paragraph_elements = Vec::new();
        
        for element in &self.spatial_elements {
            let content = element.content.trim();
            let is_in_table_region = element.vpos >= 409.0 && element.vpos <= 517.0;
            let is_table_content = content.contains('$') ||
                                  content == "N/A" ||
                                  content.contains('%') ||
                                  (content.chars().all(|c| c.is_numeric()) && content.len() == 4);
            
            if is_in_table_region && is_table_content {
                table_elements.push(element);
            } else {
                paragraph_elements.push(element);
            }
        }
        
        // Green text REMOVED for clean editing
        // for element in table_elements {
        //     let pos = egui::Pos2::new(element.hpos * scale_x, element.vpos * scale_y);
        //     painter.text(pos, egui::Align2::LEFT_TOP, &element.content, 
        //                 egui::FontId::monospace(12.0), egui::Color32::from_rgb(150, 255, 150));
        // }
        
        // Update coordinate transform with current viewport (altoedit-2.0 approach)
        let viewport_rect = response.rect;
        self.spatial_buffer.viewport_to_document_transform.update_viewport(viewport_rect);
        
        // Position text at document origin, transformed to viewport coordinates  
        let document_origin = egui::Pos2::new(20.0, 20.0); // Document space coordinates
        let start_pos = self.spatial_buffer.viewport_to_document_transform.document_to_screen(document_origin);
        
        // Render live editable text positioned relative to viewport
        self.render_live_readable_paragraphs(&painter, scale_x, scale_y, start_pos);
        
        // altoedit-2.0 style cursor positioning using grid lookup and coordinate transform
        if response.clicked() {
            if let Some(click_pos) = response.interact_pointer_pos() {
                // Convert screen click to document coordinates (like their getRealPos)
                let doc_pos = self.spatial_buffer.viewport_to_document_transform.screen_to_document(click_pos);
                
                // Use grid-based lookup for precise element detection
                if let Some(_element_idx) = self.spatial_buffer.spatial_index.find_element_at_position(doc_pos) {
                    // Found specific element - position cursor within it
                    // For now, use line-based positioning but with viewport-relative coordinates
                    let relative_x = (click_pos.x - start_pos.x).max(0.0);
                    let relative_y = (click_pos.y - start_pos.y).max(0.0);
                    
                    let live_text = self.spatial_buffer.rope.to_string();
                    let lines: Vec<&str> = live_text.lines().collect();
                    
                    let clicked_line = (relative_y / 18.0) as usize;
                    
                    if clicked_line < lines.len() {
                        let line_text = lines[clicked_line];
                        let char_in_line = (relative_x / 7.8) as usize;
                        let char_position = char_in_line.min(line_text.len());
                        
                        // Calculate rope position
                        let mut rope_pos = 0;
                        for i in 0..clicked_line {
                            rope_pos += lines[i].len() + 1;
                        }
                        rope_pos += char_position;
                        
                        self.spatial_cursor.rope_pos = rope_pos.min(self.spatial_buffer.rope.len_chars());
                    } else {
                        self.spatial_cursor.rope_pos = self.spatial_buffer.rope.len_chars();
                    }
                } else {
                    // No element in grid - still try to position cursor based on click
                    let relative_x = (click_pos.x - start_pos.x).max(0.0);
                    let relative_y = (click_pos.y - start_pos.y).max(0.0);
                    
                    let live_text = self.spatial_buffer.rope.to_string();
                    let lines: Vec<&str> = live_text.lines().collect();
                    let clicked_line = (relative_y / 18.0) as usize;
                    
                    if clicked_line < lines.len() {
                        let line_text = lines[clicked_line];
                        let char_in_line = (relative_x / 7.8) as usize;
                        let char_position = char_in_line.min(line_text.len());
                        
                        let mut rope_pos = 0;
                        for i in 0..clicked_line {
                            rope_pos += lines[i].len() + 1;
                        }
                        rope_pos += char_position;
                        
                        self.spatial_cursor.rope_pos = rope_pos.min(self.spatial_buffer.rope.len_chars());
                    } else {
                        self.spatial_cursor.rope_pos = self.spatial_buffer.rope.len_chars();
                    }
                }
                
                // Start selection
                self.selection_start = Some(self.spatial_cursor.rope_pos);
                self.selection_end = None;
            }
        }
        
        // Handle drag selection
        if response.dragged() {
            if let Some(drag_pos) = response.interact_pointer_pos() {
                let text_start = start_pos;
                let relative_x = (drag_pos.x - text_start.x).max(0.0);
                let relative_y = (drag_pos.y - text_start.y).max(0.0);
                
                let line = (relative_y / 18.0) as usize;
                let column = (relative_x / 7.8) as usize;
                let rope_pos = (line * 80) + column;
                
                self.selection_end = Some(rope_pos.min(self.spatial_buffer.rope.len_chars()));
            }
        }
        
        // ALWAYS render cursor - simplified and robust
        let live_text = self.spatial_buffer.rope.to_string();
        let lines: Vec<&str> = live_text.lines().collect();
        
        // Get cursor position with bounds checking
        let (cursor_line, cursor_char_in_line) = if lines.is_empty() {
            (0, 0) // Default if no text
        } else {
            let (line, char) = self.get_cursor_line_char(&lines);
            (line.min(lines.len().saturating_sub(1)), char) // Clamp to valid range
        };
        
        // Always render cursor at calculated position
        let cursor_screen_pos = egui::Pos2::new(
            start_pos.x + (cursor_char_in_line as f32 * 7.8),
            start_pos.y + (cursor_line as f32 * 18.0)
        );
        
        // ALWAYS draw red cursor (no conditions)
        painter.line_segment(
            [cursor_screen_pos, cursor_screen_pos + egui::Vec2::new(0.0, 16.0)],
            egui::Stroke::new(3.0, egui::Color32::RED) // Thicker for better visibility
        );
        
        // Handle text editing
        ui.input(|i| {
            for event in &i.events {
                match event {
                    egui::Event::Text(text) => {
                        self.spatial_buffer.insert_text(self.spatial_cursor.rope_pos, text);
                        self.spatial_cursor.rope_pos += text.chars().count();
                        self.modified = true;
                        
                        // XML will update automatically on next frame
                    }
                    egui::Event::Key { key, pressed: true, .. } => {
                        match key {
                            egui::Key::Backspace => {
                                if self.spatial_cursor.rope_pos > 0 {
                                    self.spatial_buffer.delete_range(self.spatial_cursor.rope_pos - 1, self.spatial_cursor.rope_pos);
                                    self.spatial_cursor.rope_pos -= 1;
                                    self.modified = true;
                                    // XML updates automatically
                                }
                            }
                            egui::Key::ArrowLeft => {
                                if self.spatial_cursor.rope_pos > 0 { self.spatial_cursor.rope_pos -= 1; }
                            }
                            egui::Key::ArrowRight => {
                                if self.spatial_cursor.rope_pos < self.spatial_buffer.rope.len_chars() { 
                                    self.spatial_cursor.rope_pos += 1; 
                                }
                            }
                            egui::Key::ArrowUp => {
                                // Claude Code style up/down navigation (respects actual line lengths)
                                let live_text = self.spatial_buffer.rope.to_string();
                                let lines: Vec<&str> = live_text.lines().collect();
                                
                                let (current_line, char_in_line) = self.get_cursor_line_char(&lines);
                                if current_line > 0 {
                                    let target_line = current_line - 1;
                                    let target_char = char_in_line.min(lines[target_line].len());
                                    self.spatial_cursor.rope_pos = self.line_char_to_rope_pos(&lines, target_line, target_char);
                                }
                            }
                            egui::Key::ArrowDown => {
                                // Claude Code style down navigation
                                let live_text = self.spatial_buffer.rope.to_string();
                                let lines: Vec<&str> = live_text.lines().collect();
                                
                                let (current_line, char_in_line) = self.get_cursor_line_char(&lines);
                                if current_line < lines.len().saturating_sub(1) {
                                    let target_line = current_line + 1;
                                    let target_char = char_in_line.min(lines[target_line].len());
                                    self.spatial_cursor.rope_pos = self.line_char_to_rope_pos(&lines, target_line, target_char);
                                }
                            }
                            egui::Key::Enter => {
                                // Add real line break for multi-line editing
                                self.spatial_buffer.insert_text(self.spatial_cursor.rope_pos, "\n");
                                self.spatial_cursor.rope_pos += 1;
                                self.modified = true;
                            }
                            egui::Key::C if i.modifiers.ctrl => {
                                // Copy selected text
                                if let (Some(start), Some(end)) = (self.selection_start, self.selection_end) {
                                    let min_pos = start.min(end);
                                    let max_pos = start.max(end);
                                    let selected_text = self.spatial_buffer.rope.slice(min_pos..max_pos).to_string();
                                    // TODO: Actually copy to system clipboard
                                }
                            }
                            egui::Key::V if i.modifiers.ctrl => {
                                // Paste from clipboard (placeholder)
                                // TODO: Get text from system clipboard and insert
                                let paste_text = "PASTED TEXT"; // Placeholder
                                self.spatial_buffer.insert_text(self.spatial_cursor.rope_pos, paste_text);
                                self.spatial_cursor.rope_pos += paste_text.len();
                                self.modified = true;
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
        });
    }
    
    fn render_live_readable_paragraphs(&mut self, painter: &egui::Painter, _scale_x: f32, _scale_y: f32, viewport_start: egui::Pos2) {
        // Show live edited rope content positioned relative to viewport
        let live_text = self.spatial_buffer.rope.to_string();
        let start_pos = viewport_start; // Use actual viewport position
        
        // Proper text formatting with natural line breaks
        let formatted_text = if live_text.contains('\n') {
            live_text // Use actual line breaks from rope
        } else {
            // Add line breaks every 80 chars only if no natural breaks exist
            live_text
                .chars()
                .collect::<Vec<char>>()
                .chunks(80)
                .map(|chunk| chunk.iter().collect::<String>())
                .collect::<Vec<String>>()
                .join("\n")
        };
        
        // Update cosmic-text buffer with current content and Kitty-quality rendering
        let kitty_attrs = Attrs::new()
            .family(Family::Name("SF Mono"))
            .weight(cosmic_text::Weight::NORMAL)
            .style(cosmic_text::Style::Normal);
        
        // Set the live text in cosmic-text buffer for superior kerning
        self.text_buffer.set_text(&mut self.font_system, &formatted_text, kitty_attrs, Shaping::Advanced);
        
        // Shape the text for proper kerning and glyph positioning
        self.text_buffer.shape_until_scroll(&mut self.font_system, false);
        
        // Render using cosmic-text's superior layout (simplified approach)
        // For now, use cosmic-text for measurement but egui for rendering
        // TODO: Full cosmic-text rendering requires more complex glyph handling
        
        // Render text with SF Mono font (Kitty-style)  
        painter.text(
            start_pos,
            egui::Align2::LEFT_TOP,
            &formatted_text,
            egui::FontId::monospace(14.0), // SF Mono-like metrics
            egui::Color32::WHITE
        );
    }
    
    fn render_live_paragraph_text(&self, painter: &egui::Painter, scale_x: f32, scale_y: f32) {
        // Render the current rope content using spatial positioning
        // This shows the LIVE edited text, not the original ALTO text
        
        for element_range in &self.spatial_buffer.element_ranges {
            // Skip table elements (they're handled separately)
            if let Some(original_element) = self.spatial_elements.get(element_range.element_id) {
                let content = original_element.content.trim();
                let is_in_table_region = original_element.vpos >= 409.0 && original_element.vpos <= 517.0;
                let is_table_content = content.contains('$') ||
                                      content == "N/A" ||
                                      content.contains('%') ||
                                      (content.chars().all(|c| c.is_numeric()) && content.len() == 4);
                
                if is_in_table_region && is_table_content {
                    continue; // Skip table elements
                }
            }
            
            // Get the current text from the spatial buffer (edited content)
            let current_text = if element_range.rope_start < self.spatial_buffer.rope.len_chars() {
                self.spatial_buffer.rope.slice(element_range.rope_start..element_range.rope_end.min(self.spatial_buffer.rope.len_chars())).to_string()
            } else {
                String::new()
            };
            
            if !current_text.is_empty() {
                let pos = egui::Pos2::new(
                    element_range.visual_bounds.min.x * scale_x,
                    element_range.visual_bounds.min.y * scale_y
                );
                
                painter.text(
                    pos,
                    egui::Align2::LEFT_TOP,
                    &current_text,
                    egui::FontId::monospace(12.0),
                    if element_range.modified {
                        egui::Color32::from_rgb(255, 200, 100) // Orange for edited
                    } else {
                        egui::Color32::WHITE
                    }
                );
            }
        }
    }
    
    fn render_readable_display(&mut self, ui: &mut egui::Ui) {
        // Use the old readable text approach that worked well
        let readable_text = self.generate_readable_text();
        
        ui.allocate_ui_with_layout(
            egui::Vec2::new(5000.0, 2000.0),  // Very wide area
            egui::Layout::top_down(egui::Align::LEFT),
            |ui| {
                ui.add(egui::Label::new(
                    egui::RichText::new(&readable_text)
                        .monospace()
                        .size(12.0)
                ));
            }
        );
        
        // Handle clicks for popup editing (old system)
        if ui.input(|i| i.pointer.any_click()) {
            if let Some(click_pos) = ui.input(|i| i.pointer.interact_pos()) {
                let clicked_element = self.find_element_at_position(click_pos, 1.2, 1.0);
                if let Some(elem_idx) = clicked_element {
                    self.editing_element = Some(elem_idx);
                    self.edit_text = self.spatial_elements[elem_idx].content.clone();
                    self.modified = true;
                }
            }
        }
    }
    
    fn get_cursor_line_char(&self, lines: &[&str]) -> (usize, usize) {
        // Calculate which line and character position cursor is on (like Claude Code)
        let mut char_count = 0;
        for (line_idx, line) in lines.iter().enumerate() {
            if self.spatial_cursor.rope_pos <= char_count + line.len() {
                return (line_idx, self.spatial_cursor.rope_pos - char_count);
            }
            char_count += line.len() + 1; // +1 for newline
        }
        (lines.len().saturating_sub(1), 0) // Default to last line, start
    }
    
    fn line_char_to_rope_pos(&self, lines: &[&str], target_line: usize, target_char: usize) -> usize {
        // Convert line/character position back to rope position
        let mut rope_pos = 0;
        for i in 0..target_line.min(lines.len()) {
            rope_pos += lines[i].len() + 1; // +1 for newline
        }
        rope_pos + target_char
    }
    
    fn generate_live_alto_xml(&self) -> String {
        // Generate real-time ALTO XML showing current editor state
        let live_text = self.spatial_buffer.rope.to_string();
        let lines: Vec<&str> = live_text.lines().collect();
        let total_lines = lines.len().max(1);
        let block_height = total_lines * 18;
        
        let mut xml = format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<alto xmlns="http://www.loc.gov/standards/alto/ns-v3#">
<Description>
  <MeasurementUnit>pixel</MeasurementUnit>
  <OCRProcessing>
    <processingSoftware>
      <softwareName>Chonker9-WYSIWYG</softwareName>
      <softwareVersion>Live Editor - {} chars, {} lines, cursor at {}</softwareVersion>
    </processingSoftware>
  </OCRProcessing>
</Description>
<Styles>
  <TextStyle ID="font0" FONTFAMILY="monospace" FONTSIZE="13.0" FONTCOLOR="FFFFFF"/>
</Styles>
<Layout>
  <Page ID="LivePage" WIDTH="800" HEIGHT="600">
    <PrintSpace>
      <TextBlock ID="live_block" HPOS="20.0" VPOS="20.0" WIDTH="640" HEIGHT="{}">
"#, live_text.chars().count(), total_lines, self.spatial_cursor.rope_pos, block_height);
        
        // Add each line as a separate TextLine with proper coordinates
        for (line_idx, line) in lines.iter().enumerate() {
            let line_y = 20.0 + (line_idx as f32 * 18.0);
            let line_width = line.len() * 8;
            
            xml.push_str(&format!(
                r#"        <TextLine ID="live_line_{}" HPOS="20.0" VPOS="{:.1}" WIDTH="{}" HEIGHT="18">
          <String ID="live_string_{}" CONTENT="{}" HPOS="20.0" VPOS="{:.1}" 
                  WIDTH="{}" HEIGHT="16" STYLEREFS="font0"/>
        </TextLine>
"#, line_idx + 1, line_y, line_width, line_idx + 1, line.replace("\"", "&quot;"), line_y, line_width));
        }
        
        xml.push_str(r#"      </TextBlock>
    </PrintSpace>
  </Page>
</Layout>
</alto>"#);
        
        xml
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
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Hot reload with Ctrl+U
        ctx.input(|i| {
            if i.key_pressed(egui::Key::U) && i.modifiers.ctrl {
                // Bootleg hot reload: quit and restart in right quadrant
                println!("🔄 Hot reloading...");
                
                // Use nohup to properly detach the process
                let spawn_result = std::process::Command::new("nohup")
                    .arg("/Users/jack/.local/bin/chonker9")
                    .arg("--right-quadrant")
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn();
                    
                match spawn_result {
                    Ok(_) => {
                        println!("✅ Hot reload spawned with nohup");
                        thread::sleep(Duration::from_millis(100));
                        std::process::exit(0);
                    }
                    Err(e) => {
                        eprintln!("❌ nohup spawn failed: {}, trying direct spawn", e);
                        // Try direct spawn with detached stdio
                        if let Ok(_) = std::process::Command::new("/Users/jack/.local/bin/chonker9")
                            .arg("--right-quadrant")
                            .stdin(std::process::Stdio::null())
                            .stdout(std::process::Stdio::null()) 
                            .stderr(std::process::Stdio::null())
                            .spawn() {
                            println!("✅ Direct spawn succeeded");
                            thread::sleep(Duration::from_millis(100));
                            std::process::exit(0);
                        } else {
                            eprintln!("❌ All spawn methods failed");
                        }
                    }
                }
            }
        });
        // Top panel with controls
        egui::TopBottomPanel::top("controls").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("📁 Load PDF").clicked() {
                    // Native file dialog for PDF selection
                    if let Some(path) = FileDialog::new()
                        .add_filter("PDF Files", &["pdf"])
                        .add_filter("All Files", &["*"])
                        .set_directory("~/Documents")
                        .pick_file() {
                        
                        self.pdf_path = path.to_string_lossy().to_string();
                        if let Err(e) = self.load_pdf() {
                            eprintln!("Error loading PDF: {}", e);
                        }
                    }
                }
                
                ui.separator();
                
                if ui.button("🔍 XML Debug").clicked() {
                    self.show_xml_debug = !self.show_xml_debug;
                }
                
                // Removed pointless button - live XML shows automatically in split view
                
                
                if self.show_xml_debug {
                    ui.label("📋 Debug Mode");
                    if ui.button("💾 Save XML").clicked() {
                        if let Err(e) = std::fs::write("chonker9_debug.xml", &self.raw_xml) {
                            eprintln!("Error saving XML: {}", e);
                        }
                    }
                } else {
                    if ui.button("💾 Save Text").clicked() {
                        let content = self.spatial_buffer.rope.to_string();
                        if let Err(e) = std::fs::write("chonker9_edited.txt", content) {
                            eprintln!("Error saving text: {}", e);
                        }
                    }
                }
            });
        });
        
        // Force true 50/50 split - left panel takes exactly half
        let screen_width = ctx.screen_rect().width();
        egui::SidePanel::left("xml_panel")
            .exact_width(screen_width / 2.0)
            .resizable(false)
            .show(ctx, |ui| {
                ui.heading("🔍 Live ALTO XML (Updates in Real-Time)");
                
                // Always show live XML (since we have split view)
                ui.label("Auto-updates as you edit →");
                
                let xml_display = self.generate_live_alto_xml();
                
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add_sized(
                            ui.available_size(),
                            egui::TextEdit::multiline(&mut xml_display.as_str())
                                .font(egui::TextStyle::Monospace)
                                .code_editor()
                        );
                    });
            });

        // Right panel: WYSIWYG Editor using full quadrant
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("📝 WYSIWYG Editor (Edit and Watch XML Update)");
            
            // Use entire remaining space for editor
            if !self.spatial_elements.is_empty() {
                self.render_wysiwyg_readable(ui);
            } else {
                ui.label("Loading content...");
            }
        });
        
        // Pure WYSIWYG spatial editing - no popups needed
    }
}

fn main() -> Result<(), eframe::Error> {
    println!("🚀 Starting Chonker9...");
    
    // Check for right quadrant positioning argument
    let args: Vec<String> = std::env::args().collect();
    let right_quadrant = args.contains(&"--right-quadrant".to_string());
    
    let mut app = ChonkerApp::default();
    
    // Auto-load the default PDF
    println!("📁 Loading PDF...");
    match app.load_pdf() {
        Ok(()) => {
            println!("✅ PDF loaded successfully - {} elements", app.spatial_elements.len());
        }
        Err(e) => {
            eprintln!("❌ Error loading PDF: {}", e);
            eprintln!("💡 Continuing without PDF data - you can load one manually");
        }
    }
    
    // Use fixed screen dimensions to avoid system calls that might cause issues
    let screen_width = 1920.0;
    let screen_height = 1080.0;
    println!("📺 Using default screen size: {}x{}", screen_width, screen_height);
    
    let (window_width, window_height, x_pos, y_pos) = if right_quadrant {
        // Right HALF of screen, full height, touching bottom
        let w = screen_width / 2.0;    // Half screen width  
        let h = screen_height;         // Full screen height (touches bottom)
        let x = screen_width / 2.0;    // Start exactly at screen center
        let y = 0.0;                   // Top of screen
        (w, h, x, y)
    } else {
        // Default positioning
        (1000.0, 700.0, 100.0, 100.0)
    };
    
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([window_width, window_height])
            .with_position([x_pos, y_pos])
            .with_title("Chonker9 - PDF Editor"),
        ..Default::default()
    };
    
    if right_quadrant {
        println!("🖥️ Creating window in right half: {}×{} at ({}, {})", window_width, window_height, x_pos, y_pos);
    } else {
        println!("🖥️ Creating window...");
    }
    
    eframe::run_native(
        "Chonker9",
        options,
        Box::new(|_cc| {
            println!("✅ Window created");
            Ok(Box::new(app))
        }),
    )
}