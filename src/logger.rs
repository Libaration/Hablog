use std::{
    fmt::{Debug, Display},
    io::Write,
};
use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};

pub struct ConsoleLogger {}

impl ConsoleLogger {
    fn print_log<T: Display>(level: &str, message: T, color: Color) {
        let mut stdout = StandardStream::stdout(ColorChoice::Always);
        let log_prefix = format!("[hablog]{}", level);

        stdout
            .set_color(ColorSpec::new().set_bold(true).set_fg(Some(color)))
            .unwrap();
        write!(&mut stdout, "{}", log_prefix).unwrap();
        stdout.set_color(&ColorSpec::new().set_reset(true)).unwrap();
        writeln!(&mut stdout, ":: {}", message).unwrap();
    }

    pub fn info<T: Display>(message: T) {
        Self::print_log("", message, Color::Blue);
    }

    pub fn success<T: Display>(message: T) {
        Self::print_log("", message, Color::Green);
    }

    pub fn warning<T: Display>(message: T) {
        Self::print_log("", message, Color::Yellow);
    }

    pub fn error<T: Debug>(error: T) {
        let error_message = format!("{:?}", error);
        let log_message = format!("[hablog]:: Error: {}", error_message);
        let mut stdout = StandardStream::stdout(ColorChoice::Always);

        stdout
            .set_color(
                ColorSpec::new()
                    .set_bold(true)
                    .set_fg(Some(Color::Red))
                    .set_intense(true)
                    .set_italic(true),
            )
            .unwrap();
        writeln!(&mut stdout, "{}", log_message).unwrap();
        stdout.set_color(&ColorSpec::new().set_reset(true)).unwrap();
    }
    pub fn print_dashes() {
        let mut stdout = StandardStream::stdout(ColorChoice::Always);
        let colors = [
            Color::Red,
            Color::Yellow,
            Color::Green,
            Color::Cyan,
            Color::Blue,
            Color::Magenta,
        ];

        for color in &colors {
            stdout
                .set_color(ColorSpec::new().set_fg(Some(*color)))
                .unwrap();
            writeln!(
                &mut stdout,
                "--------------------------------------------------------------------"
            )
            .unwrap();
        }

        stdout.set_color(&ColorSpec::new().set_reset(true)).unwrap();
    }
    pub fn print_rainbow_text<T: Display>(message: T) {
        let mut stdout = StandardStream::stdout(ColorChoice::Always);
        let colors = [
            Color::Red,
            Color::Yellow,
            Color::Green,
            Color::Cyan,
            Color::Blue,
            Color::Magenta,
        ];
        let rainbow_chars = format!("[hablog]:: {}", message)
            .chars()
            .enumerate()
            .collect::<Vec<_>>();

        for (index, c) in rainbow_chars {
            let color = colors[index % colors.len()];
            stdout
                .set_color(
                    ColorSpec::new()
                        .set_bold(true)
                        .set_underline(true)
                        .set_fg(Some(color)),
                )
                .unwrap();
            write!(&mut stdout, "{}", c).unwrap();
        }

        writeln!(&mut stdout).unwrap();

        let binary_data = r#"
         01100001 01101010 01101001 01101110 00100000 01101001 01110011 00100000
         01100001 00100000 01100010 01101001 01101110 01100001 01110010 01111001
         00100000 01100100 01100001 01110100 01100001 00101100 00100000 01110011
         01110101 01110010 01100101 00101100 00100000 01101110 01101111 00111111
        "#;

        let binary_data_lines = binary_data.trim().lines();

        for (index, line) in binary_data_lines.enumerate() {
            let color = colors[index % colors.len()];
            stdout
                .set_color(ColorSpec::new().set_bold(true).set_fg(Some(color)))
                .unwrap();
            writeln!(
                &mut stdout,
                "{}",
                line.replace(" ", "").replace("00100000", " ")
            )
            .unwrap();
        }
    }

    pub fn custom<T: Display>(level: &str, message: T, color: Color) {
        Self::print_log(level, message, color);
    }

    pub fn normal<T: Display>(message: T) {
        Self::print_log("", message, Color::White);
    }
}
