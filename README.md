# Chonker9 - Advanced Terminal PDF Viewer

A lightweight, high-performance terminal-based PDF viewer with spatial text extraction using pdfalto.

## Features

- **ALTO XML Processing**: Uses pdfalto for high-quality PDF text extraction
- **Spatial Layout**: Preserves original document positioning and formatting
- **Terminal Display**: Clean, text-based output with proper spacing
- **Reading Order**: Follows visual reading order for natural text flow
- **Lightweight**: Minimal dependencies, fast processing

## Requirements

- Rust 1.70 or later
- `pdfalto` (from poppler-utils)

### Installing Dependencies

**macOS:**
```bash
brew install poppler
```

**Ubuntu/Debian:**
```bash
sudo apt-get install poppler-utils
```

**Arch Linux:**
```bash
sudo pacman -S poppler
```

## Installation

```bash
# Clone the repository
git clone https://github.com/jackgrauer/chonker9.git
cd chonker9

# Build the project
cargo build --release

# Run the viewer
./target/release/chonker9 path/to/your.pdf
```

## Usage

```bash
# Open a PDF file
./target/release/chonker9 document.pdf

# If no file specified, uses default test PDF
./target/release/chonker9
```

## Architecture

Chonker9 is built with a minimal, focused architecture:

- **PDF Processing**: Uses pdfalto for ALTO XML extraction
- **Spatial Parsing**: Processes XML to extract text positioning
- **Terminal Rendering**: Converts coordinates to terminal positioning
- **Line Reconstruction**: Groups text elements into natural reading lines

## Version History

### v9.0.0 (Current)
- Complete rewrite focused on pdfalto integration
- Spatial text layout preservation
- Minimal terminal-based interface
- High-performance ALTO XML processing

## License

MIT License - See LICENSE file for details

## Author

Jack Grauer ([@jackgrauer](https://github.com/jackgrauer))