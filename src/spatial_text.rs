// spatial_text.rs - Core WYSIWYG spatial text editing system
use eframe::egui;
use ropey::Rope;
use std::collections::HashMap;

/// Coordinate transformation system (altoedit-2.0 getRealPos approach)
#[derive(Debug, Clone)]
pub struct CoordinateTransform {
    pub viewport_rect: egui::Rect,    // Current viewport bounds
    pub document_rect: egui::Rect,    // Document bounds in document space  
    pub scale: f32,                   // Current zoom scale
}

impl CoordinateTransform {
    pub fn new() -> Self {
        Self {
            viewport_rect: egui::Rect::NOTHING,
            document_rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::new(800.0, 600.0)),
            scale: 1.0,
        }
    }
    
    pub fn update_viewport(&mut self, viewport_rect: egui::Rect) {
        self.viewport_rect = viewport_rect;
    }
    
    pub fn screen_to_document(&self, screen_pos: egui::Pos2) -> egui::Pos2 {
        // Convert viewport coordinates to document coordinates (altoedit-2.0 getRealPos)
        let relative_pos = screen_pos - self.viewport_rect.min;
        egui::pos2(relative_pos.x / self.scale, relative_pos.y / self.scale)
    }
    
    pub fn document_to_screen(&self, doc_pos: egui::Pos2) -> egui::Pos2 {
        // Convert document coordinates to viewport coordinates  
        let scaled_pos = egui::pos2(doc_pos.x * self.scale, doc_pos.y * self.scale);
        scaled_pos + self.viewport_rect.min.to_vec2()
    }
}

/// Maps a range in the unified text buffer to spatial positioning
#[derive(Debug, Clone)]
pub struct ElementRange {
    pub rope_start: usize,        // Start position in unified rope
    pub rope_end: usize,          // End position in unified rope
    pub element_id: usize,        // Original ALTO element index
    pub visual_bounds: egui::Rect, // Current display bounds
    pub original_bounds: egui::Rect, // Original ALTO bounds
    pub overflow: bool,           // Text exceeds original bounds
    pub modified: bool,           // Has been edited from original
}

/// Grid-based spatial indexing (like altoedit-2.0)
#[derive(Debug)]
pub struct SpatialIndex {
    grid: Vec<Vec<Vec<usize>>>,              // grid[y][x] = [element_indices]
    grid_size: f32,                          // Size of each grid cell
    doc_bounds: egui::Rect,                  // Document bounds for grid calculation
    dirty_regions: Vec<egui::Rect>,          // Regions needing re-render
}

impl SpatialIndex {
    pub fn new() -> Self {
        Self {
            grid: Vec::new(),
            grid_size: 50.0, // 50px grid cells (like altoedit-2.0)
            doc_bounds: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::new(1000.0, 1000.0)),
            dirty_regions: Vec::new(),
        }
    }
    
    pub fn rebuild(&mut self, element_ranges: &[ElementRange]) {
        // Build grid-based spatial index (altoedit-2.0 approach)
        
        // Calculate document bounds
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;
        
        for range in element_ranges {
            let bounds = &range.visual_bounds;
            min_x = min_x.min(bounds.min.x);
            min_y = min_y.min(bounds.min.y);
            max_x = max_x.max(bounds.max.x);
            max_y = max_y.max(bounds.max.y);
        }
        
        self.doc_bounds = egui::Rect::from_min_max(
            egui::pos2(min_x, min_y),
            egui::pos2(max_x, max_y)
        );
        
        // Initialize grid
        let grid_cols = ((self.doc_bounds.width() / self.grid_size).ceil() as usize).max(1);
        let grid_rows = ((self.doc_bounds.height() / self.grid_size).ceil() as usize).max(1);
        
        self.grid = vec![vec![Vec::new(); grid_cols]; grid_rows];
        
        // Populate grid with element indices
        for (i, range) in element_ranges.iter().enumerate() {
            let bounds = &range.visual_bounds;
            
            // Find which grid cells this element overlaps
            let start_col = ((bounds.min.x - self.doc_bounds.min.x) / self.grid_size) as usize;
            let end_col = ((bounds.max.x - self.doc_bounds.min.x) / self.grid_size) as usize;
            let start_row = ((bounds.min.y - self.doc_bounds.min.y) / self.grid_size) as usize;
            let end_row = ((bounds.max.y - self.doc_bounds.min.y) / self.grid_size) as usize;
            
            // Add element to all overlapping grid cells
            for row in start_row..=end_row.min(grid_rows.saturating_sub(1)) {
                for col in start_col..=end_col.min(grid_cols.saturating_sub(1)) {
                    self.grid[row][col].push(i);
                }
            }
        }
    }
    
    pub fn find_element_at_position(&self, pos: egui::Pos2) -> Option<usize> {
        // Fast grid-based lookup (altoedit-2.0 technique)
        
        // Check if position is within document bounds
        if !self.doc_bounds.contains(pos) {
            return None;
        }
        
        // Calculate grid cell
        let col = ((pos.x - self.doc_bounds.min.x) / self.grid_size) as usize;
        let row = ((pos.y - self.doc_bounds.min.y) / self.grid_size) as usize;
        
        // Check grid bounds
        if row >= self.grid.len() || col >= self.grid[row].len() {
            return None;
        }
        
        // Search only elements in this grid cell (much faster than linear search)
        for &element_idx in &self.grid[row][col] {
            // Return first match (could be enhanced with distance calculation)
            return Some(element_idx);
        }
        
        None
    }
    
    pub fn mark_dirty_region(&mut self, bounds: egui::Rect) {
        self.dirty_regions.push(bounds);
    }
    
    pub fn clear_dirty_regions(&mut self) {
        self.dirty_regions.clear();
    }
}

/// Main spatial text buffer that bridges linear editing and 2D layout
#[derive(Debug)]
pub struct SpatialTextBuffer {
    pub rope: Rope,                           // Unified text buffer
    pub element_ranges: Vec<ElementRange>,    // Maps rope ranges to spatial positions
    pub spatial_index: SpatialIndex,         // Fast spatial queries
    pub cursor_pos: usize,                   // Current cursor position in rope
    pub selection: Option<(usize, usize)>,   // Selection range in rope
    pub zoom: f32,                           // Current zoom level
    pub pan: egui::Vec2,                     // Current pan offset
    // Coordinate transformation system (altoedit-2.0 style)
    pub viewport_to_document_transform: CoordinateTransform,
}

impl SpatialTextBuffer {
    pub fn new() -> Self {
        Self {
            rope: Rope::new(),
            element_ranges: Vec::new(),
            spatial_index: SpatialIndex::new(),
            cursor_pos: 0,
            selection: None,
            zoom: 1.0,
            pan: egui::Vec2::ZERO,
            viewport_to_document_transform: CoordinateTransform::new(),
        }
    }
    
    /// Build from ALTO spatial elements 
    pub fn from_alto_elements(elements: &[(String, f32, f32, f32, f32)]) -> Self {
        let mut buffer = Self::new();
        let mut rope_text = String::new();
        let mut char_pos = 0;
        
        // Build rope respecting ALTO structure (TextBlocks and TextLines)
        let mut current_line_vpos = -1.0;
        
        for (i, (content, hpos, vpos, width, height)) in elements.iter().enumerate() {
            let start_pos = char_pos;
            
            // Add line break when VPOS changes significantly (new TextLine in ALTO)
            if i > 0 && (vpos - current_line_vpos).abs() > 5.0 {
                rope_text.push('\n');
                char_pos += 1;
                current_line_vpos = *vpos;
            } else if i == 0 {
                current_line_vpos = *vpos;
            }
            
            rope_text.push_str(content);
            char_pos += content.chars().count();
            
            // Add space between elements on same line
            if i < elements.len() - 1 {
                let next_vpos = elements[i + 1].2;
                if (next_vpos - vpos).abs() <= 5.0 {
                    // Same line - add space
                    rope_text.push(' ');
                    char_pos += 1;
                }
            }
            
            let end_pos = char_pos;
            
            // Create element range mapping
            let element_range = ElementRange {
                rope_start: start_pos,
                rope_end: end_pos,
                element_id: i,
                visual_bounds: egui::Rect::from_min_size(
                    egui::pos2(*hpos, *vpos), 
                    egui::vec2(*width, *height)
                ),
                original_bounds: egui::Rect::from_min_size(
                    egui::pos2(*hpos, *vpos), 
                    egui::vec2(*width, *height)
                ),
                overflow: false,
                modified: false,
            };
            
            buffer.element_ranges.push(element_range);
        }
        
        // Build rope and index
        buffer.rope = Rope::from_str(&rope_text);
        buffer.spatial_index.rebuild(&buffer.element_ranges);
        
        buffer
    }
    
    /// Convert screen click to rope position
    pub fn screen_to_rope_position(&self, screen_pos: egui::Pos2) -> Option<usize> {
        // Transform screen coordinates to document coordinates
        let doc_pos = self.screen_to_document_pos(screen_pos);
        
        // Find element at position
        if let Some(element_idx) = self.spatial_index.find_element_at_position(doc_pos) {
            let element = &self.element_ranges[element_idx];
            
            // Calculate position within element
            let local_pos = doc_pos - element.visual_bounds.min;
            
            // Better character positioning that accounts for accumulation error
            let element_text_len = (element.rope_end - element.rope_start) as f32;
            
            let char_offset = if element_text_len > 0.0 {
                // Use proportional positioning instead of fixed char width
                let relative_x = local_pos.x / element.visual_bounds.width();
                ((relative_x * element_text_len) as usize).min(element_text_len as usize)
            } else {
                0
            };
            
            Some(element.rope_start + char_offset)
        } else {
            None
        }
    }
    
    /// Convert rope position to screen coordinates
    pub fn rope_to_screen_position(&self, rope_pos: usize) -> Option<egui::Pos2> {
        // Find which element contains this rope position
        for element in &self.element_ranges {
            if rope_pos >= element.rope_start && rope_pos < element.rope_end {
                let char_offset = rope_pos - element.rope_start;
                let element_text_len = element.rope_end - element.rope_start;
                
                // Calculate position within element with matching offset compensation
                let char_width = if element_text_len > 0 {
                    element.visual_bounds.width() / element_text_len as f32
                } else {
                    8.0
                };
                let local_x = (char_offset as f32 * char_width) + 5.0; // Apply same offset compensation
                
                // Transform to screen coordinates
                let doc_pos = element.visual_bounds.min + egui::vec2(local_x, 0.0);
                return Some(self.document_to_screen_pos(doc_pos));
            }
        }
        None
    }
    
    /// Screen coordinate transformations
    fn screen_to_document_pos(&self, screen_pos: egui::Pos2) -> egui::Pos2 {
        (screen_pos - self.pan) / self.zoom
    }
    
    fn document_to_screen_pos(&self, doc_pos: egui::Pos2) -> egui::Pos2 {
        doc_pos * self.zoom + self.pan
    }
    
    /// Insert text at rope position and update spatial mappings
    pub fn insert_text(&mut self, pos: usize, text: &str) {
        let insert_len = text.chars().count();
        
        // Insert into rope
        self.rope.insert(pos, text);
        
        // Update all element ranges after the insertion point
        for element in &mut self.element_ranges {
            if element.rope_start > pos {
                element.rope_start += insert_len;
                element.rope_end += insert_len;
            } else if element.rope_end > pos {
                element.rope_end += insert_len;
                element.modified = true;
                
                // Check for overflow (defer text_exceeds_bounds call to avoid borrow issues)
                element.overflow = true; // Mark for later overflow check
            }
        }
        
        // Second pass: check overflow for modified elements
        let mut overflow_checks = Vec::new();
        for (i, element) in self.element_ranges.iter().enumerate() {
            if element.modified && element.overflow {
                let current_text = self.rope.slice(element.rope_start..element.rope_end).to_string();
                overflow_checks.push((i, self.text_exceeds_bounds(&current_text, &element.original_bounds)));
            }
        }
        
        // Apply overflow results
        for (i, overflow_result) in overflow_checks {
            self.element_ranges[i].overflow = overflow_result;
        }
        
        // Mark affected region as dirty
        if let Some(element) = self.find_element_containing_position(pos) {
            self.spatial_index.mark_dirty_region(element.visual_bounds);
        }
    }
    
    /// Delete text range and update spatial mappings
    pub fn delete_range(&mut self, start: usize, end: usize) {
        let delete_len = end - start;
        
        // Delete from rope
        self.rope.remove(start..end);
        
        // Update element ranges
        for element in &mut self.element_ranges {
            if element.rope_start > end {
                element.rope_start -= delete_len;
                element.rope_end -= delete_len;
            } else if element.rope_end > start {
                // Element is affected by deletion
                if element.rope_start >= start {
                    // Element starts within deleted range
                    element.rope_start = start;
                }
                if element.rope_end > end {
                    element.rope_end -= delete_len;
                } else {
                    element.rope_end = start;
                }
                element.modified = true;
            }
        }
        
        // Rebuild spatial index
        self.spatial_index.rebuild(&self.element_ranges);
    }
    
    fn find_element_containing_position(&self, rope_pos: usize) -> Option<&ElementRange> {
        self.element_ranges.iter().find(|e| rope_pos >= e.rope_start && rope_pos < e.rope_end)
    }
    
    fn text_exceeds_bounds(&self, text: &str, bounds: &egui::Rect) -> bool {
        // Simple width check - can be enhanced with cosmic-text measurement
        let estimated_width = text.len() as f32 * 8.0; // Assume 8px per character
        estimated_width > bounds.width()
    }
}

/// Visual cursor that tracks spatial position
#[derive(Debug)]
pub struct SpatialCursor {
    pub rope_pos: usize,
    pub screen_pos: Option<egui::Pos2>,
    pub blink_timer: std::time::Instant,
    pub visible: bool,
}

impl SpatialCursor {
    pub fn new() -> Self {
        Self {
            rope_pos: 0,
            screen_pos: None,
            blink_timer: std::time::Instant::now(),
            visible: true,
        }
    }
    
    pub fn update_position(&mut self, buffer: &SpatialTextBuffer) {
        self.screen_pos = buffer.rope_to_screen_position(self.rope_pos);
        
        // Update blink state
        if self.blink_timer.elapsed().as_millis() > 500 {
            self.visible = !self.visible;
            self.blink_timer = std::time::Instant::now();
        }
    }
    
    pub fn render(&self, painter: &egui::Painter) {
        if let Some(pos) = self.screen_pos {
            if self.visible {
                painter.line_segment(
                    [pos, pos + egui::vec2(0.0, 15.0)],
                    egui::Stroke::new(2.0, egui::Color32::from_rgb(40, 90, 200))
                );
            }
        }
    }
    
    pub fn move_to_rope_position(&mut self, pos: usize, buffer: &SpatialTextBuffer) {
        self.rope_pos = pos.min(buffer.rope.len_chars());
        self.update_position(buffer);
    }
    
    pub fn move_to_screen_position(&mut self, screen_pos: egui::Pos2, buffer: &SpatialTextBuffer) {
        if let Some(rope_pos) = buffer.screen_to_rope_position(screen_pos) {
            self.rope_pos = rope_pos;
            self.screen_pos = Some(screen_pos);
        }
    }
}