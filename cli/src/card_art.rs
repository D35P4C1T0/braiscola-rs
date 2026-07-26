use std::collections::HashMap;
use std::path::{Path, PathBuf};

use briscola_core::card::{Card, Rank, Suit};
use image::{ImageReader, RgbaImage, imageops::FilterType};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

pub struct AsciiCardRenderer {
    cache: HashMap<Card, Vec<String>>,
    max_width: usize,
}

impl AsciiCardRenderer {
    pub fn new(max_width: usize) -> Self {
        Self { cache: HashMap::new(), max_width }
    }

    pub fn max_width(&self) -> usize {
        self.max_width
    }

    pub fn render_card(&mut self, card: Card) -> Result<Vec<String>, String> {
        if let Some(lines) = self.cache.get(&card) {
            return Ok(lines.clone());
        }

        let lines = self.render_card_uncached(card)?;
        self.cache.insert(card, lines.clone());
        Ok(lines)
    }

    fn render_card_uncached(&self, card: Card) -> Result<Vec<String>, String> {
        let image_path = card_asset_path(card);
        let image = ImageReader::open(&image_path)
            .map_err(|error| format!("cannot open card image '{}': {error}", image_path.display()))?
            .decode()
            .map_err(|error| {
                format!("cannot decode card image '{}': {error}", image_path.display())
            })?
            .to_rgba8();

        let Some((min_x, min_y, max_x, max_y)) = non_background_bounds(&image) else {
            return Err(format!("card image '{}' has no visible pixels", image_path.display()));
        };

        let crop_w = max_x - min_x + 1;
        let crop_h = max_y - min_y + 1;
        let cropped = image::imageops::crop_imm(&image, min_x, min_y, crop_w, crop_h).to_image();

        let target_w_u32 =
            u32::try_from(self.max_width).map_err(|_| String::from("invalid target width"))?;
        let mut target_h =
            crop_h.saturating_mul(target_w_u32).saturating_mul(55) / crop_w.saturating_mul(100);
        target_h = target_h.clamp(8, 22);

        let resized =
            image::imageops::resize(&cropped, target_w_u32, target_h, FilterType::Triangle);
        let shades: &[u8] = b"@%#*+=-:.";
        let shades_last = u32::try_from(shades.len().saturating_sub(1))
            .map_err(|_| String::from("invalid shade table"))?;

        let capacity =
            usize::try_from(target_h).map_err(|_| String::from("invalid target height"))?;
        let mut lines = Vec::with_capacity(capacity);
        for y in 0..target_h {
            let mut line = String::with_capacity(self.max_width);
            for x in 0..target_w_u32 {
                let pixel = resized.get_pixel(x, y);
                if is_background(pixel.0) {
                    line.push(' ');
                    continue;
                }

                let red = u32::from(pixel.0[0]);
                let green = u32::from(pixel.0[1]);
                let blue = u32::from(pixel.0[2]);
                let luminance = (2126_u32.saturating_mul(red)
                    + 7152_u32.saturating_mul(green)
                    + 722_u32.saturating_mul(blue))
                    / 10_000;

                if luminance >= 245 {
                    line.push(' ');
                    continue;
                }

                let inverted = 255_u32.saturating_sub(luminance);
                let shade_index_u32 = inverted.saturating_mul(shades_last) / 255;
                let shade_index = usize::try_from(shade_index_u32)
                    .map_err(|_| String::from("invalid shade index"))?;
                line.push(char::from(shades[shade_index]));
            }
            lines.push(line);
        }

        Ok(lines)
    }
}

pub struct TerminalCardRenderer {
    cache: HashMap<Card, Vec<Line<'static>>>,
    max_width: usize,
}

impl TerminalCardRenderer {
    pub fn new(max_width: usize) -> Self {
        Self { cache: HashMap::new(), max_width }
    }

    pub fn render_card(&mut self, card: Card) -> Result<Vec<Line<'static>>, String> {
        if let Some(lines) = self.cache.get(&card) {
            return Ok(lines.clone());
        }

        let lines = self.render_card_uncached(card)?;
        self.cache.insert(card, lines.clone());
        Ok(lines)
    }

    fn render_card_uncached(&self, card: Card) -> Result<Vec<Line<'static>>, String> {
        let image_path = card_asset_path(card);
        let image = ImageReader::open(&image_path)
            .map_err(|error| format!("cannot open card image '{}': {error}", image_path.display()))?
            .decode()
            .map_err(|error| {
                format!("cannot decode card image '{}': {error}", image_path.display())
            })?
            .to_rgba8();

        let Some((min_x, min_y, max_x, max_y)) = non_background_bounds(&image) else {
            return Err(format!("card image '{}' has no visible pixels", image_path.display()));
        };

        let crop_w = max_x - min_x + 1;
        let crop_h = max_y - min_y + 1;
        let cropped = image::imageops::crop_imm(&image, min_x, min_y, crop_w, crop_h).to_image();

        let target_w_u32 =
            u32::try_from(self.max_width).map_err(|_| String::from("invalid target width"))?;
        let target_h = terminal_target_height(crop_w, crop_h, target_w_u32);

        let resized =
            image::imageops::resize(&cropped, target_w_u32, target_h, FilterType::CatmullRom);

        let mut lines = Vec::new();
        let mut y = 0_u32;
        while y < target_h {
            let mut spans = Vec::with_capacity(self.max_width);
            for x in 0..target_w_u32 {
                let top = resized.get_pixel(x, y).0;
                let bottom = resized.get_pixel(x, y + 1).0;
                let top_background = is_background(top);
                let bottom_background = is_background(bottom);

                let span = match (top_background, bottom_background) {
                    (true, true) => Span::raw(" "),
                    (false, true) => Span::styled("▀", Style::default().fg(pixel_to_color(top))),
                    (true, false) => Span::styled("▄", Style::default().fg(pixel_to_color(bottom))),
                    (false, false) => Span::styled(
                        "▀",
                        Style::default().fg(pixel_to_color(top)).bg(pixel_to_color(bottom)),
                    ),
                };
                spans.push(span);
            }
            lines.push(Line::from(spans));
            y = y.saturating_add(2);
        }

        Ok(lines)
    }
}

fn terminal_target_height(source_width: u32, source_height: u32, target_width: u32) -> u32 {
    // Each terminal row packs two vertical image pixels into a half-block. Keeping
    // the image-pixel aspect ratio here compensates for typical tall terminal cells
    // once those pixel pairs are collapsed into rows.
    let mut height = source_height.saturating_mul(target_width) / source_width;
    height = height.clamp(8, 28);
    if !height.is_multiple_of(2) {
        height = height.saturating_add(1);
    }
    height
}

pub fn card_name_english(card: Card) -> String {
    format!("{} of {}", rank_english(card.rank), suit_english(card.suit))
}

pub fn card_name_italian(card: Card) -> String {
    format!("{} di {}", rank_italian(card.rank), suit_italian(card.suit))
}

pub fn card_name_bilingual(card: Card) -> String {
    format!("{} | {}", card_name_english(card), card_name_italian(card))
}

fn card_asset_path(card: Card) -> PathBuf {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../res/napoletane");
    base.join(card_asset_name(card))
}

fn card_asset_name(card: Card) -> String {
    let suit_name = match card.suit {
        Suit::Coins => "denara",
        Suit::Cups => "coppe",
        Suit::Swords => "spade",
        Suit::Clubs => "bastoni",
    };

    let rank_number = match card.rank {
        Rank::Ace => 1_u8,
        Rank::Two => 2_u8,
        Rank::Three => 3_u8,
        Rank::Four => 4_u8,
        Rank::Five => 5_u8,
        Rank::Six => 6_u8,
        Rank::Seven => 7_u8,
        Rank::Jack => 8_u8,
        Rank::Queen => 9_u8,
        Rank::King => 10_u8,
    };

    format!("{suit_name}{rank_number}.webp")
}

fn non_background_bounds(image: &RgbaImage) -> Option<(u32, u32, u32, u32)> {
    let width = image.width();
    let height = image.height();
    if width == 0 || height == 0 {
        return None;
    }

    let mut found = false;
    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0_u32;
    let mut max_y = 0_u32;

    for y in 0..height {
        for x in 0..width {
            let pixel = image.get_pixel(x, y);
            if is_background(pixel.0) {
                continue;
            }

            found = true;
            if x < min_x {
                min_x = x;
            }
            if y < min_y {
                min_y = y;
            }
            if x > max_x {
                max_x = x;
            }
            if y > max_y {
                max_y = y;
            }
        }
    }

    if found { Some((min_x, min_y, max_x, max_y)) } else { None }
}

fn is_background(pixel: [u8; 4]) -> bool {
    let alpha = pixel[3];
    let red = pixel[0];
    let green = pixel[1];
    let blue = pixel[2];
    alpha < 16 || (red > 245 && green > 245 && blue > 245)
}

fn pixel_to_color(pixel: [u8; 4]) -> Color {
    Color::Rgb(pixel[0], pixel[1], pixel[2])
}

fn rank_english(rank: Rank) -> &'static str {
    match rank {
        Rank::Ace => "Ace",
        Rank::Two => "Two",
        Rank::Three => "Three",
        Rank::Four => "Four",
        Rank::Five => "Five",
        Rank::Six => "Six",
        Rank::Seven => "Seven",
        Rank::Jack => "Jack",
        Rank::Queen => "Queen",
        Rank::King => "King",
    }
}

fn rank_italian(rank: Rank) -> &'static str {
    match rank {
        Rank::Ace => "Asso",
        Rank::Two => "Due",
        Rank::Three => "Tre",
        Rank::Four => "Quattro",
        Rank::Five => "Cinque",
        Rank::Six => "Sei",
        Rank::Seven => "Sette",
        Rank::Jack => "Fante",
        Rank::Queen => "Cavallo",
        Rank::King => "Re",
    }
}

fn suit_english(suit: Suit) -> &'static str {
    match suit {
        Suit::Coins => "Coins",
        Suit::Cups => "Cups",
        Suit::Swords => "Swords",
        Suit::Clubs => "Clubs",
    }
}

fn suit_italian(suit: Suit) -> &'static str {
    match suit {
        Suit::Coins => "Denari",
        Suit::Cups => "Coppe",
        Suit::Swords => "Spade",
        Suit::Clubs => "Bastoni",
    }
}

#[cfg(test)]
mod tests {
    use briscola_core::card::{Card, Rank, Suit};

    use super::{TerminalCardRenderer, terminal_target_height};

    #[test]
    fn half_block_target_height_keeps_terminal_cards_compact() {
        assert_eq!(terminal_target_height(100, 200, 14), 28);
        assert_eq!(terminal_target_height(100, 200, 10), 20);
    }

    #[test]
    fn rendered_hand_card_is_no_more_than_fourteen_rows_tall() {
        let mut renderer = TerminalCardRenderer::new(14);
        let lines = renderer.render_card(Card::new(Suit::Clubs, Rank::King)).expect("render card");

        assert!(!lines.is_empty());
        assert!(lines.len() <= 14);
    }
}
