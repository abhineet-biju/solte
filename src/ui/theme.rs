use ratatui::style::Color;

#[derive(Clone, Copy)]
pub struct Theme {
    pub bg: Color,
    pub panel: Color,
    pub border: Color,
    pub text: Color,
    pub muted: Color,
    pub accent: Color,
    pub green: Color,
    pub red: Color,
    pub selected: Color,
}

impl Theme {
    pub fn named(name: &str) -> Self {
        let mut theme = Self {
            bg: Color::Rgb(12, 16, 18),
            panel: Color::Rgb(16, 22, 25),
            border: Color::Rgb(49, 64, 70),
            text: Color::Rgb(213, 224, 228),
            muted: Color::Rgb(120, 142, 152),
            accent: Color::Rgb(244, 191, 79),
            green: Color::Rgb(68, 222, 151),
            red: Color::Rgb(250, 111, 119),
            selected: Color::Rgb(26, 53, 45),
        };
        match name {
            "glacier" => {
                theme.accent = Color::Rgb(117, 196, 245);
                theme.selected = Color::Rgb(26, 48, 65);
            }
            "orchid" => {
                theme.accent = Color::Rgb(199, 157, 246);
                theme.selected = Color::Rgb(46, 36, 64);
            }
            _ => {}
        }
        theme
    }
}
