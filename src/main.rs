use std::{process::Command, io::{self, Write}};
use crossterm::{
    cursor, 
    event::{self, KeyCode, Event, MouseEventKind, MouseButton, EnableMouseCapture, DisableMouseCapture},
    terminal::{enable_raw_mode, disable_raw_mode, Clear, ClearType},
    ExecutableCommand,
};
use ropey::Rope;
use cosmic_text::{FontSystem, Buffer, Metrics};

#[derive(Debug, Clone)]
struct SpatialElement {
    start_char: usize,  // Position in rope
    end_char: usize,    // End position in rope
    hpos: f32,
    vpos: f32,
    width: f32,
    height: f32,
}

#[derive(Debug, Clone)]
struct Selection {
    start: usize,  // Selection start position in rope
    end: usize,    // Selection end position in rope
    active: bool,  // Whether selection is active
}

impl Selection {
    fn new() -> Self {
        Self { start: 0, end: 0, active: false }
    }
    
    fn start_selection(&mut self, pos: usize) {
        self.start = pos;
        self.end = pos;
        self.active = true;
    }
    
    fn extend_selection(&mut self, pos: usize) {
        if self.active {
            self.end = pos;
        }
    }
    
    fn clear(&mut self) {
        self.active = false;
    }
    
    fn get_range(&self) -> (usize, usize) {
        if self.start <= self.end {
            (self.start, self.end)
        } else {
            (self.end, self.start)
        }
    }
}

struct EditableDocument {
    rope: Rope,                           // Primary text buffer
    spatial_map: Vec<SpatialElement>,     // Maps text positions to spatial coords
    lines: Vec<Vec<usize>>,               // Line -> [spatial_element_indices]
    cursor_pos: usize,                    // Cursor position in rope (char index)
    selection: Selection,                 // Text selection state
    modified: bool,
    save_confirmed: bool,
    font_system: FontSystem,
    cosmic_buffer: Buffer,
}

impl EditableDocument {
    fn new(parsed_lines: Vec<Vec<(String, f32, f32, f32, f32)>>) -> Self {
        let mut rope_text = String::new();
        let mut spatial_map = Vec::new();
        let mut lines = Vec::new();
        let mut char_pos = 0;
        
        // Build rope text and spatial mapping
        for (line_idx, line) in parsed_lines.iter().enumerate() {
            let mut line_elements = Vec::new();
            
            for (elem_idx, (content, hpos, vpos, width, height)) in line.iter().enumerate() {
                let start_char = char_pos;
                
                // Add content to rope text
                rope_text.push_str(content);
                char_pos += content.chars().count();
                
                // Add space between elements (except last in line)
                if elem_idx < line.len() - 1 {
                    rope_text.push(' ');
                    char_pos += 1;
                }
                
                let end_char = char_pos;
                
                // Create spatial element
                let spatial_elem = SpatialElement {
                    start_char,
                    end_char,
                    hpos: *hpos,
                    vpos: *vpos,
                    width: *width,
                    height: *height,
                };
                
                spatial_map.push(spatial_elem);
                line_elements.push(spatial_map.len() - 1); // Store index
            }
            
            lines.push(line_elements);
            
            // Add newline between lines (except last)
            if line_idx < parsed_lines.len() - 1 {
                rope_text.push('\n');
                char_pos += 1;
            }
        }
        
        let rope = Rope::from_str(&rope_text);
        let mut font_system = FontSystem::new();
        let mut cosmic_buffer = Buffer::new(&mut font_system, Metrics::new(14.0, 16.0));
        
        // Set up cosmic-text buffer
        cosmic_buffer.set_text(&mut font_system, &rope_text, cosmic_text::Attrs::new(), cosmic_text::Shaping::Advanced);
        
        Self {
            rope,
            spatial_map,
            lines,
            cursor_pos: 0,
            selection: Selection::new(),
            modified: false,
            save_confirmed: false,
            font_system,
            cosmic_buffer,
        }
    }
    
    fn move_cursor_up(&mut self) {
        // Ensure cursor is within bounds
        self.cursor_pos = self.cursor_pos.min(self.rope.len_chars());
        
        let line_idx = self.rope.char_to_line(self.cursor_pos);
        if line_idx > 0 {
            let prev_line_start = self.rope.line_to_char(line_idx - 1);
            let prev_line = self.rope.line(line_idx - 1);
            let prev_line_len = prev_line.len_chars().saturating_sub(1); // Exclude newline
            let current_col = self.cursor_pos - self.rope.line_to_char(line_idx);
            
            // Try to maintain column position, but clamp to line length
            let new_col = current_col.min(prev_line_len);
            self.cursor_pos = prev_line_start + new_col;
        }
    }
    
    fn move_cursor_down(&mut self) {
        // Ensure cursor is within bounds
        self.cursor_pos = self.cursor_pos.min(self.rope.len_chars());
        
        let line_idx = self.rope.char_to_line(self.cursor_pos);
        if line_idx < self.rope.len_lines().saturating_sub(1) {
            let next_line_start = self.rope.line_to_char(line_idx + 1);
            let next_line = self.rope.line(line_idx + 1);
            let next_line_len = next_line.len_chars().saturating_sub(1); // Exclude newline
            let current_col = self.cursor_pos - self.rope.line_to_char(line_idx);
            
            // Try to maintain column position, but clamp to line length
            let new_col = current_col.min(next_line_len);
            self.cursor_pos = next_line_start + new_col;
        }
    }
    
    fn move_cursor_left(&mut self) {
        if self.cursor_pos > 0 {
            self.cursor_pos = self.cursor_pos.saturating_sub(1);
        }
    }
    
    fn move_cursor_right(&mut self) {
        if self.cursor_pos < self.rope.len_chars() {
            self.cursor_pos += 1;
        }
    }
    
    fn update_cosmic_buffer(&mut self) {
        // Safely update cosmic-text buffer with current rope content
        let text = self.rope.to_string();
        self.cosmic_buffer.set_text(&mut self.font_system, &text, cosmic_text::Attrs::new(), cosmic_text::Shaping::Advanced);
    }
    
    fn terminal_to_rope_pos(&self, col: u16, row: u16) -> usize {
        // Find the line that best matches the clicked row
        let mut best_line_idx = 0;
        let mut min_distance = u16::MAX;
        
        for (line_idx, line_element_indices) in self.lines.iter().enumerate() {
            if let Some(&first_elem_idx) = line_element_indices.first() {
                let first_elem = &self.spatial_map[first_elem_idx];
                let term_row = ((first_elem.vpos / 12.0) as u16).max(1);
                let distance = if row >= term_row { row - term_row } else { term_row - row };
                
                if distance < min_distance {
                    min_distance = distance;
                    best_line_idx = line_idx;
                }
            }
        }
        
        // Find position within the line
        let line_start = self.rope.line_to_char(best_line_idx);
        let line = self.rope.line(best_line_idx);
        let line_len = line.len_chars().saturating_sub(1); // Exclude newline
        
        // Get spatial info for the line to calculate column offset
        if let Some(line_element_indices) = self.lines.get(best_line_idx) {
            if let Some(&first_elem_idx) = line_element_indices.first() {
                let first_elem = &self.spatial_map[first_elem_idx];
                let term_col_start = ((first_elem.hpos / 8.0) as u16).max(1);
                
                // Calculate approximate character position based on click column
                let clicked_col_offset = if col >= term_col_start { 
                    col - term_col_start 
                } else { 
                    0 
                };
                
                let char_pos = (clicked_col_offset as usize).min(line_len);
                return line_start + char_pos;
            }
        }
        
        // Fallback to line start
        line_start
    }
    
    fn insert_char(&mut self, c: char) {
        // Ensure cursor is within valid bounds
        let cursor_pos = self.cursor_pos.min(self.rope.len_chars());
        
        self.rope.insert_char(cursor_pos, c);
        self.cursor_pos = cursor_pos + 1;
        self.modified = true;
        
        // Update cosmic-text buffer safely
        self.update_cosmic_buffer();
    }
    
    fn delete_char(&mut self) {
        if self.cursor_pos > 0 && self.cursor_pos <= self.rope.len_chars() {
            let prev_pos = self.cursor_pos.saturating_sub(1);
            
            // Ensure we don't remove beyond bounds
            if prev_pos < self.rope.len_chars() && self.cursor_pos <= self.rope.len_chars() {
                self.rope.remove(prev_pos..self.cursor_pos);
                self.cursor_pos = prev_pos;
                self.modified = true;
                
                // Update cosmic-text buffer safely
                self.update_cosmic_buffer();
            }
        }
    }
    
    fn render(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // Clear screen and home cursor
        io::stdout().execute(Clear(ClearType::All))?.execute(cursor::MoveTo(0, 0))?;
        
        // Use cosmic-text for advanced text shaping and layout
        self.cosmic_buffer.shape_until_scroll(&mut self.font_system, false);
        
        // Ensure cursor is within bounds before rendering
        self.cursor_pos = self.cursor_pos.min(self.rope.len_chars());
        
        let cursor_line = self.rope.char_to_line(self.cursor_pos);
        let cursor_col = self.cursor_pos - self.rope.line_to_char(cursor_line);
        let mut cursor_terminal_row = 1;
        let mut cursor_terminal_col = 1;
        
        // Render each line using spatial information where available
        for (line_idx, line_element_indices) in self.lines.iter().enumerate() {
            if line_element_indices.is_empty() { continue; }
            
            let rope_line = self.rope.line(line_idx);
            let line_text = rope_line.to_string().trim_end().to_string();
            
            // Get spatial info from first element for positioning
            if let Some(&first_elem_idx) = line_element_indices.first() {
                let first_elem = &self.spatial_map[first_elem_idx];
                
                // Calculate terminal position
                let term_row = ((first_elem.vpos / 12.0) as u16).max(1);
                let term_col = ((first_elem.hpos / 8.0) as u16).max(1);
                
                // Calculate line range in rope
                let line_start = self.rope.line_to_char(line_idx);
                let line_end = line_start + line_text.chars().count();
                
                // Check if this line has selection
                if self.selection.active {
                    let (sel_start, sel_end) = self.selection.get_range();
                    
                    // If line intersects with selection, render with highlighting
                    if sel_start < line_end && sel_end > line_start {
                        print!("\x1b[{};{}H", term_row, term_col);
                        
                        for (char_idx, ch) in line_text.chars().enumerate() {
                            let char_pos = line_start + char_idx;
                            
                            if char_pos >= sel_start && char_pos < sel_end {
                                print!("\x1b[7m{}\x1b[0m", ch); // Highlighted character
                            } else {
                                print!("{}", ch); // Normal character
                            }
                        }
                    } else {
                        // No selection on this line
                        print!("\x1b[{};{}H{}", term_row, term_col, line_text);
                    }
                } else {
                    // No active selection
                    print!("\x1b[{};{}H{}", term_row, term_col, line_text);
                }
                
                // Calculate cursor position if this is the active line
                if line_idx == cursor_line {
                    cursor_terminal_row = term_row;
                    cursor_terminal_col = term_col + cursor_col as u16;
                }
            }
        }
        
        // Position the cursor at the calculated location and show visual indicator
        print!("\x1b[{};{}H", cursor_terminal_row, cursor_terminal_col);
        print!("\x1b[7m \x1b[0m"); // Print inverted space as cursor
        io::stdout().execute(cursor::MoveTo(cursor_terminal_col, cursor_terminal_row))?;
        
        // Calculate status position - find the bottom of document content
        let max_vpos = self.spatial_map.iter()
            .map(|e| e.vpos)
            .fold(0.0, f32::max);
        let content_bottom = ((max_vpos / 12.0) as u16) + 3;
        
        // Get terminal size to ensure status doesn't go off screen
        let (_terminal_width, terminal_height) = crossterm::terminal::size().unwrap_or((80, 24));
        let status_row = (content_bottom.max(terminal_height.saturating_sub(2))).min(terminal_height - 1);
        
        let cursor_line = self.rope.char_to_line(self.cursor_pos);
        let cursor_col = self.cursor_pos - self.rope.line_to_char(cursor_line);
        
        // Show status at safe bottom position
        print!("\x1b[{};1H", status_row);
        print!("📝 Chonker9 Editor (Ropey+Cosmic+Mouse) - Line {}, Col {} ", 
               cursor_line + 1, cursor_col + 1);
        
        if self.selection.active {
            let (start, end) = self.selection.get_range();
            let sel_len = end - start;
            print!("| SEL: {} chars ", sel_len);
        }
        
        print!("| ");
        if self.save_confirmed { 
            print!("✅ SAVED | ");
            self.save_confirmed = false; // Clear after showing once
        } else if self.modified { 
            print!("*MODIFIED* | "); 
        }
        print!("Click/Drag: Select | Shift+Arrows: Select | Ctrl+A: All | Ctrl+S: Save | Ctrl+Q: Quit");
        
        io::stdout().flush()?;
        Ok(())
    }
    
    fn save_to_file(&mut self, filename: &str) -> Result<(), Box<dyn std::error::Error>> {
        // Save the current rope content directly
        let content = self.rope.to_string();
        std::fs::write(filename, content)?;
        self.modified = false;
        self.save_confirmed = true;
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Chonker9 - Advanced Terminal PDF Viewer (pdfalto edition)");
    
    // Get PDF path from command line args or use default
    let args: Vec<String> = std::env::args().collect();
    let pdf_path = if args.len() > 1 {
        &args[1]
    } else {
        "/Users/jack/Documents/chonker_test.pdf"
    };
    
    println!("📁 Processing: {}", std::fs::canonicalize(pdf_path)?.display());
    
    // Extract PDF using pdfalto with optimal flags
    let xml = extract_pdf_xml(pdf_path)?;
    
    // Parse and create editable document
    let text_elements = parse_xml_spatially(&xml)?;
    let lines = group_into_lines(text_elements);
    
    // Start interactive editing session
    start_editor(lines)?;
    
    Ok(())
}

fn extract_pdf_xml(pdf_path: &str) -> Result<String, Box<dyn std::error::Error>> {
    let output = Command::new("pdfalto")
        .args([
            "-f", "1", "-l", "1",   // Just page 1 for now
            "-readingOrder",        // Follow visual reading order
            "-noImage",            // Skip image extraction for speed
            "-noLineNumbers",      // Clean output without line numbers
            pdf_path, 
            "/dev/stdout"
        ])
        .output()?;
    
    if !output.status.success() {
        return Err("pdfalto failed".into());
    }
    
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn parse_xml_spatially(xml: &str) -> Result<Vec<(String, f32, f32, f32, f32)>, Box<dyn std::error::Error>> {
    use quick_xml::{Reader, events::Event};
    
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    let mut text_elements = Vec::new();
    
    println!("🎯 Parsing ALTO XML...");
    
    let mut in_page = false;
    let mut page_count = 0;
    
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let tag_bytes = e.name().as_ref().to_vec();
                let tag_name = String::from_utf8_lossy(&tag_bytes);
                
                if tag_name == "Page" {
                    in_page = true;
                    page_count += 1;
                    println!("📄 Found Page {}", page_count);
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
                        text_elements.push((content, hpos, vpos, width, height));
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
    
    println!("✅ Found {} pages, extracted {} text elements", 
        page_count, text_elements.len());
    
    Ok(text_elements)
}

fn group_into_lines(text_elements: Vec<(String, f32, f32, f32, f32)>) -> Vec<Vec<(String, f32, f32, f32, f32)>> {
    // Group text elements into lines (within 8 pixels vertically)
    let mut lines: Vec<Vec<(String, f32, f32, f32, f32)>> = Vec::new();
    let mut sorted_elements = text_elements.clone();
    sorted_elements.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap()); // Sort by VPOS
    
    for element in sorted_elements {
        let found_line = lines.iter_mut().find(|line| {
            if let Some(first) = line.first() {
                (element.2 - first.2).abs() < 8.0  // Within 8 pixels vertically
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
    
    // Sort words within each line by HPOS (left to right)
    for line in &mut lines {
        line.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    }
    
    println!("📐 Lines reconstructed: {}", lines.len());
    lines
}

fn start_editor(lines: Vec<Vec<(String, f32, f32, f32, f32)>>) -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = EditableDocument::new(lines);
    
    // Enable raw mode and mouse capture
    enable_raw_mode()?;
    io::stdout().execute(EnableMouseCapture)?;
    
    // Initial render
    doc.render()?;
    
    // Main editor loop
    loop {
        match event::read()? {
            Event::Key(key) => {
                match key.code {
                    KeyCode::Char('q') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                        break;
                    }
                    KeyCode::Char('s') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                        doc.save_to_file("chonker9_edited.txt")?;
                        // Save confirmation will show in next render cycle via status bar
                    }
                    KeyCode::Char('a') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                        // Select all
                        doc.selection.start_selection(0);
                        doc.selection.extend_selection(doc.rope.len_chars());
                    }
                    KeyCode::Char('c') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                        // Copy selection (placeholder - would need clipboard integration)
                        if doc.selection.active {
                            let (start, end) = doc.selection.get_range();
                            let selected_text = doc.rope.slice(start..end).to_string();
                            // TODO: Copy to clipboard
                        }
                    }
                    KeyCode::Char(c) => {
                        if doc.selection.active {
                            // Replace selection with new character - with bounds checking
                            let (start, end) = doc.selection.get_range();
                            let rope_len = doc.rope.len_chars();
                            
                            if start <= rope_len && end <= rope_len && start <= end {
                                doc.rope.remove(start..end);
                                doc.rope.insert_char(start, c);
                                doc.cursor_pos = start + 1;
                                doc.selection.clear();
                                doc.modified = true;
                                
                                // Update cosmic-text buffer
                                doc.update_cosmic_buffer();
                            }
                        } else {
                            doc.insert_char(c);
                        }
                    }
                    KeyCode::Backspace => {
                        if doc.selection.active {
                            // Delete selection - with bounds checking
                            let (start, end) = doc.selection.get_range();
                            let rope_len = doc.rope.len_chars();
                            
                            if start <= rope_len && end <= rope_len && start <= end {
                                doc.rope.remove(start..end);
                                doc.cursor_pos = start;
                                doc.selection.clear();
                                doc.modified = true;
                                
                                // Update cosmic-text buffer
                                doc.update_cosmic_buffer();
                            }
                        } else {
                            doc.delete_char();
                        }
                    }
                    KeyCode::Up => {
                        if key.modifiers.contains(event::KeyModifiers::SHIFT) {
                            // Extend selection up
                            if !doc.selection.active {
                                doc.selection.start_selection(doc.cursor_pos);
                            }
                            doc.move_cursor_up();
                            doc.selection.extend_selection(doc.cursor_pos);
                        } else {
                            doc.selection.clear();
                            doc.move_cursor_up();
                        }
                    }
                    KeyCode::Down => {
                        if key.modifiers.contains(event::KeyModifiers::SHIFT) {
                            // Extend selection down
                            if !doc.selection.active {
                                doc.selection.start_selection(doc.cursor_pos);
                            }
                            doc.move_cursor_down();
                            doc.selection.extend_selection(doc.cursor_pos);
                        } else {
                            doc.selection.clear();
                            doc.move_cursor_down();
                        }
                    }
                    KeyCode::Left => {
                        if key.modifiers.contains(event::KeyModifiers::SHIFT) {
                            // Extend selection left
                            if !doc.selection.active {
                                doc.selection.start_selection(doc.cursor_pos);
                            }
                            doc.move_cursor_left();
                            doc.selection.extend_selection(doc.cursor_pos);
                        } else {
                            doc.selection.clear();
                            doc.move_cursor_left();
                        }
                    }
                    KeyCode::Right => {
                        if key.modifiers.contains(event::KeyModifiers::SHIFT) {
                            // Extend selection right
                            if !doc.selection.active {
                                doc.selection.start_selection(doc.cursor_pos);
                            }
                            doc.move_cursor_right();
                            doc.selection.extend_selection(doc.cursor_pos);
                        } else {
                            doc.selection.clear();
                            doc.move_cursor_right();
                        }
                    }
                    KeyCode::Home => {
                        // Move to beginning of line
                        let line_idx = doc.rope.char_to_line(doc.cursor_pos);
                        let line_start = doc.rope.line_to_char(line_idx);
                        
                        if key.modifiers.contains(event::KeyModifiers::SHIFT) {
                            if !doc.selection.active {
                                doc.selection.start_selection(doc.cursor_pos);
                            }
                            doc.cursor_pos = line_start;
                            doc.selection.extend_selection(doc.cursor_pos);
                        } else {
                            doc.selection.clear();
                            doc.cursor_pos = line_start;
                        }
                    }
                    KeyCode::End => {
                        // Move to end of line
                        let line_idx = doc.rope.char_to_line(doc.cursor_pos);
                        let line_start = doc.rope.line_to_char(line_idx);
                        let line = doc.rope.line(line_idx);
                        let line_len = line.len_chars().saturating_sub(1); // Exclude newline
                        
                        if key.modifiers.contains(event::KeyModifiers::SHIFT) {
                            if !doc.selection.active {
                                doc.selection.start_selection(doc.cursor_pos);
                            }
                            doc.cursor_pos = line_start + line_len;
                            doc.selection.extend_selection(doc.cursor_pos);
                        } else {
                            doc.selection.clear();
                            doc.cursor_pos = line_start + line_len;
                        }
                    }
                    KeyCode::Esc => {
                        if doc.selection.active {
                            doc.selection.clear();
                        } else {
                            break;
                        }
                    }
                    _ => {}
                }
                
                // Re-render after each action
                doc.render()?;
            }
            Event::Mouse(mouse) => {
                match mouse.kind {
                    MouseEventKind::Down(MouseButton::Left) => {
                        // Start selection or move cursor
                        let new_pos = doc.terminal_to_rope_pos(mouse.column, mouse.row);
                        doc.cursor_pos = new_pos;
                        doc.selection.start_selection(new_pos);
                        doc.render()?;
                    }
                    MouseEventKind::Drag(MouseButton::Left) => {
                        // Extend selection while dragging
                        let new_pos = doc.terminal_to_rope_pos(mouse.column, mouse.row);
                        doc.cursor_pos = new_pos;
                        doc.selection.extend_selection(new_pos);
                        doc.render()?;
                    }
                    MouseEventKind::Up(MouseButton::Left) => {
                        // End selection
                        if doc.selection.start == doc.selection.end {
                            doc.selection.clear(); // Clear if no actual selection was made
                        }
                        doc.render()?;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    
    // Clean up
    disable_raw_mode()?;
    io::stdout().execute(DisableMouseCapture)?;
    
    // Clear screen and show final message
    io::stdout().execute(Clear(ClearType::All))?.execute(cursor::MoveTo(0, 0))?;
    println!("📄 Chonker9 Editor session ended");
    if doc.modified {
        println!("💾 Don't forget to save with Ctrl+S if you want to keep changes!");
    }
    
    Ok(())
}