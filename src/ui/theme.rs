use ratatui::style::{Color, Modifier, Style};

#[derive(Clone, Copy)]
pub struct Theme {
    pub bg: Color,
    pub panel: Color,
    pub border: Color,
    pub text: Color,
    pub muted: Color,
    pub accent: Color,
    pub heading: Color,
    pub green: Color,
    pub red: Color,
    pub selected: Color,
}

impl Theme {
    pub fn control(self) -> Style {
        Style::default().fg(self.text).bg(self.selected)
    }

    pub fn selected_control(self) -> Style {
        self.control().fg(self.accent).add_modifier(Modifier::BOLD)
    }

    pub fn focused_control(self) -> Style {
        Style::default()
            .fg(self.bg)
            .bg(self.accent)
            .add_modifier(Modifier::BOLD)
            .remove_modifier(Modifier::UNDERLINED | Modifier::REVERSED | Modifier::DIM)
    }

    pub fn input(self, focused: bool) -> Style {
        if focused {
            self.focused_control()
        } else {
            self.control()
        }
    }

    pub fn named(name: &str) -> Self {
        let mut theme = Self {
            bg: Color::Rgb(12, 16, 18),
            panel: Color::Rgb(16, 22, 25),
            border: Color::Rgb(49, 64, 70),
            text: Color::Rgb(213, 224, 228),
            muted: Color::Rgb(120, 142, 152),
            accent: Color::Rgb(244, 191, 79),
            heading: Color::Rgb(244, 191, 79),
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
            "neon" => {
                theme.bg = Color::Rgb(5, 10, 18);
                theme.panel = Color::Rgb(7, 16, 28);
                theme.border = Color::Rgb(24, 65, 98);
                theme.text = Color::Rgb(203, 214, 250);
                theme.muted = Color::Rgb(133, 153, 187);
                theme.accent = Color::Rgb(236, 91, 247);
                theme.heading = Color::Rgb(39, 222, 238);
                theme.green = Color::Rgb(35, 231, 188);
                theme.red = Color::Rgb(255, 99, 140);
                theme.selected = Color::Rgb(17, 38, 58);
            }
            _ => {}
        }
        if name != "neon" {
            theme.heading = theme.accent;
        }
        theme
    }
}
