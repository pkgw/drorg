// Copyright 2018 Peter Williams <peter@newton.cx>
// Licensed under the MIT License.

//! Structured, colorized printing to the terminal using
//! [termcolor](https://github.com/BurntSushi/termcolor).
//!
//! **Real docs TODO**.
//!
//! This module is designed to be helpful without requiring any special code,
//! but it is extensible if you want to add features.
//!
//! ```
//! use tcprint::{BasicColors, ColorPrintState};
//! let mut state = ColorPrintState::<BasicColors>::default();
//!
//! let q = 17;
//! tcprint!(state, [red: "oh no:"], ("q is: {}", q));
//! ```

#![deny(missing_docs)]

extern crate termcolor;

use std::default::Default;
use std::fmt;
use std::io::{self, Write};
use termcolor::{ColorChoice, StandardStream, WriteColor};

pub use termcolor::{Color, ColorSpec};


/// Which destination to print text to: standard output or standard error.
///
/// This enum may seem a bit superfluous, but it's possible that we might want
/// to extend it with additional variants in the future.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrintDestination {
    /// Print to standard error.
    Stderr,

    /// Print to standard output.
    Stdout,
}


/// A structure capturing access to all output streams.
///
/// Users of this crate shouldn't need to care about this type, but it needs
/// to be made public so that the underlying macros can work. So it is hidden.
#[doc(hidden)]
pub struct PrintStreams {
    stdout: StandardStream,
    stderr: StandardStream,
}

impl Default for PrintStreams {
    fn default() -> Self {
        let stdout = StandardStream::stdout(ColorChoice::Auto);
        let stderr = StandardStream::stderr(ColorChoice::Auto);

        PrintStreams {
            stdout,
            stderr,
        }
    }
}

impl PrintStreams {
    /// Print colorized output to one (or more) of the output streams.
    ///
    /// This is a low-level function, expected to be used by higher-level APIs.
    #[inline(always)]
    pub fn print_color(&mut self, stream: PrintDestination, color: &ColorSpec, args: fmt::Arguments)
        -> Result<(), io::Error>
    {
        let stream = match stream {
            PrintDestination::Stderr => &mut self.stderr,
            PrintDestination::Stdout => &mut self.stdout,
        };

        stream.set_color(&color)?;
        let r = write!(stream, "{}", args);
        stream.reset()?;
        r
    }


    /// Print to one (or more) of the output streams without changing the colorization.
    ///
    /// This is a low-level function, expected to be used by higher-level APIs.
    #[inline(always)]
    pub fn print_nocolor(&mut self, stream: PrintDestination, args: fmt::Arguments)
        -> Result<(), io::Error>
    {
        let stream = match stream {
            PrintDestination::Stderr => &mut self.stderr,
            PrintDestination::Stdout => &mut self.stdout,
        };

        write!(stream, "{}", args)
    }
}


/// A basic selection of colors for printing to the terminal.
pub struct BasicColors {
    /// Bold red.
    pub red: ColorSpec,

    /// Bold green.
    pub green: ColorSpec,

    /// Bold yellow.
    pub yellow: ColorSpec,

    /// "Highlight": bold white .
    pub hl: ColorSpec,
}

impl Default for BasicColors {
    fn default() -> Self {
        let mut green = ColorSpec::new();
        green.set_fg(Some(Color::Green)).set_bold(true);

        let mut red = ColorSpec::new();
        red.set_fg(Some(Color::Red)).set_bold(true);

        let mut yellow = ColorSpec::new();
        yellow.set_fg(Some(Color::Yellow)).set_bold(true);

        let mut hl = ColorSpec::new();
        hl.set_bold(true);

        BasicColors {
            green,
            red,
            yellow,
            hl,
        }
    }
}


/// State for colorized printing.
///
/// The type parameter `C` should be a structure containing public fields of
/// type `termcolor::ColorSpec`. These will be accessed inside the macros to
/// simplify the creation of colorized output.
#[derive(Default)]
pub struct ColorPrintState<C> {
    streams: PrintStreams,
    colors: C,
}

impl<C> ColorPrintState<C> {
    /// Initialize colorized printing state.
    pub fn new(colors: C) -> Self {
        let streams = PrintStreams::default();
        ColorPrintState { streams, colors }
    }

    /// Work around borrowck/macro issues.
    #[doc(hidden)]
    pub fn split_into_components_mut<'a>(&'a mut self) -> (&'a mut PrintStreams, &'a C) {
        (&mut self.streams, &self.colors)
    }
}


#[macro_export]
macro_rules! tcanyprint {
    (@clause $cps:expr, $dest:expr, [$color:ident : $($fmt_args:expr),*]) => {{
        let (streams, colors) = $cps.split_into_components_mut();
        let _r = streams.print_color($dest, &colors.$color, format_args!($($fmt_args),*));
    }};

    (@clause $cps:expr, $dest:expr, ($($fmt_args:expr),*)) => {{
        let (streams, _colors) = $cps.split_into_components_mut();
        let _r = streams.print_nocolor($dest, format_args!($($fmt_args),*));
    }};

    ($cps:expr, $dest:expr, $($clause:tt),*) => {{
        $(
            tcanyprint!(@clause $cps, $dest, $clause);
        )*
    }};
}


#[macro_export]
macro_rules! tcprint {
    ($cps:expr, $($clause:tt),*) => {{
        use $crate::PrintDestination;
        tcanyprint!($cps, PrintDestination::Stdout, $($clause),*)
    }};
}


#[macro_export]
macro_rules! etcprint {
    ($cps:expr, $($clause:tt),*) => {{
        use $crate::PrintDestination;
        tcanyprint!($cps, PrintDestination::Stderr, $($clause),*)
    }};
}


#[macro_export]
macro_rules! tcprintln {
    ($cps:expr, $($clause:tt),*) => {{
        tcprint!($cps, $($clause),*, ("\n"))
    }};
}


#[macro_export]
macro_rules! etcprintln {
    ($cps:expr, $($clause:tt),*) => {{
        etcprint!($cps, $($clause),*, ("\n"))
    }};
}


/// A helper enumeration defining differen "report" types.
///
/// TODO: Ord, PartialOrd?
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReportType {
    /// An informational message.
    Note,

    /// A warning.
    Warning,

    /// An error.
    Error,
}


/// A helper trait for accessing the colors associated with standard "report" types.
pub trait ReportingColors {
    /// Get a `termcolor::ColorSpec` to be associated with a report message.
    fn get_color_for_report(&self, reptype: ReportType) -> &ColorSpec;
}

impl ReportingColors for BasicColors {
    fn get_color_for_report(&self, reptype: ReportType) -> &ColorSpec {
        match reptype {
            ReportType::Note => &self.green,
            ReportType::Warning => &self.yellow,
            ReportType::Error => &self.red,
        }
    }
}

#[macro_export]
macro_rules! tcreport {
    (@inner $cps:expr, $type:expr, $prefix:expr, $($fmt_args:expr),*) => {{
        {
            use $crate::{PrintDestination, ReportingColors};
            let (streams, colors) = $cps.split_into_components_mut();
            let color = colors.get_color_for_report($type);
            let _r = streams.print_color(PrintDestination::Stdout, color, format_args!($prefix));
        }

        tcprintln!($cps, (" "), ($($fmt_args),*));
    }};

    ($cps:expr, note : $($fmt_args:expr),*) => {{
        use $crate::ReportType;
        tcreport!(@inner $cps, ReportType::Note, "note:", $($fmt_args),*)
    }};

    ($cps:expr, warning : $($fmt_args:expr),*) => {{
        use $crate::ReportType;
        tcreport!(@inner $cps, ReportType::Warning, "warning:", $($fmt_args),*)
    }};

    ($cps:expr, error : $($fmt_args:expr),*) => {{
        use $crate::ReportType;
        tcreport!(@inner $cps, ReportType::Error, "error:", $($fmt_args),*)
    }};
}
