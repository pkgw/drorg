// Copyright 2018 Peter Williams <peter@newton.cx>
// Licensed under the MIT License.

//! Structured, colorized printing to the terminal using [termcolor].
//!
//! The [termcolor] crate has been carefully designed to allow CLI tools to
//! print colors to the terminal in a cross-platform fashion — while most
//! color-print crates only work with Unix color codes, [termcolor] also works
//! on Windows. While this is a valuable capability, the [termcolor] API is
//! fairly low-level.
//!
//! This crate provides a slightly higher-level interface that aims to be
//! convenient for basic use cases, and extensible when needed. First of all,
//! the relevant state is gathered into a single `ColorPrintState` structure
//! that can be passed around your application. This comprises (1) handles to
//! color-capable standard output and error streams and (2) a palette of
//! pre-defined colors. Second, macros are provided that make it easier to
//! print output mixing a variety of colors.
//!
//! ## Basic Usage
//!
//! ```
//! #[macro_use] extern crate tcprint;
//!
//! use tcprint::{BasicColors, ColorPrintState};
//!
//! let mut state = ColorPrintState::<BasicColors>::default();
//! let q = 17;
//! tcprintln!(state, [red: "oh no:"], (" q is: {}", q));
//! ```
//!
//! The above will print the line `oh no: q is 17`, where the phrase `oh no:`
//! will appear in red. The arguments to the `tcprintln!` macro are structured
//! as:
//!
//! ```ignore
//! tcprintln!(state_object, clause1, ...clauseN);
//! ```
//!
//! Where each clause takes on one of the following forms:
//!
//! - `(format, args...)` to print without applying colorization
//! - `[colorname: format, args...]` to print applying the named color
//!   (see `BasicColors` for a list of what’s available in the simple case)
//! - `{color_var, {block}: format, args...}` to print applying a color that
//!   is determined on-the-fly, potentially using local variables to choose
//!   the color (see `tcprint!()` for examples)
//!
//! Along with `tcprintln!()`, macros named `tcprint!()`, `etcprintln!()`, and
//! `etcprint!()` are provided, all in analogy with the printing macros
//! provided with the Rust standard library.
//!
//! ## Log-Style Messages
//!
//! An additional macro named `tcreport!()` is provided to ease the printing
//! of log messages classified as "info", "warning", or "error". **TODO:
//! should play nice with the standard log API!**:
//!
//! ```
//! # #[macro_use] extern crate tcprint;
//! # use tcprint::{BasicColors, ColorPrintState};
//! # let mut state = ColorPrintState::<BasicColors>::default();
//! tcreport!(state, warning: "could not locate puppy");
//! ```
//!
//! This will emit the text `warning: could not locate puppy`, where the
//! portion `warning:` appears in bold yellow by default. Other allowed
//! prefixes are `info:` (appearing in green) and `error:` (appearing in red).
//!
//! ## Custom Palettes
//!
//! To use a custom palette of colors, define your own struct with public
//! fields of type `termcolor::ColorSpec`. Then use that struct instead of
//! `BasicColors` when creating the `ColorPrintState` struct. This crate
//! re-exports `Color` and `ColorSpec` from `termcolor` for convenience in
//! doing so.
//!
//! ```
//! #[macro_use] extern crate tcprint;
//!
//! use std::default::Default;
//! use tcprint::{Color, ColorSpec, ColorPrintState};
//!
//! #[derive(Clone, Debug, Eq, PartialEq)]
//! struct MyPalette {
//!     /// In this app, pet names should always be printed using this color specification.
//!     pub pet_name: ColorSpec,
//! }
//!
//! impl Default for MyPalette {
//!     fn default() -> Self {
//!         // By default, pet names are printed in bold blue.
//!         let mut pet_name = ColorSpec::new();
//!         pet_name.set_fg(Some(Color::Blue)).set_bold(true);
//!
//!         MyPalette { pet_name }
//!     }
//! }
//!
//! fn main() {
//!     let mut state = ColorPrintState::<MyPalette>::default();
//!
//!     let name = "Quemmy";
//!     tcprintln!(state,
//!          ("the name of my dog is "),
//!          [pet_name: "{}", name],
//!          ("!")
//!     );
//! }
//! ```
//!
//! If you want to use `tcreport!()` with your custom palette, it must
//! implement the `ReportingColors` trait.
//!
//! **TODO**: figure out locking plan!
//!
//! [termcolor]: https://github.com/BurntSushi/termcolor

#![deny(missing_docs)]

extern crate termcolor;

use std::default::Default;
use std::fmt;
use std::io::{self, Write};
use termcolor::{ColorChoice, StandardStream, WriteColor};

#[doc(no_inline)]
pub use termcolor::{Color, ColorSpec};

/// Which destination to print text to: standard output or standard error.
///
/// This enum may seem a bit superfluous, but it's possible that we might want
/// to extend it with additional variants in the future (e.g., to print to
/// both streams, or something).
#[doc(hidden)]
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

        PrintStreams { stdout, stderr }
    }
}

impl PrintStreams {
    /// Print colorized output to one (or more) of the output streams.
    ///
    /// This is a low-level function, expected to be used by higher-level APIs.
    #[inline(always)]
    pub fn print_color(
        &mut self,
        stream: PrintDestination,
        color: &ColorSpec,
        args: fmt::Arguments,
    ) -> io::Result<()> {
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
    pub fn print_nocolor(
        &mut self,
        stream: PrintDestination,
        args: fmt::Arguments,
    ) -> io::Result<()> {
        let stream = match stream {
            PrintDestination::Stderr => &mut self.stderr,
            PrintDestination::Stdout => &mut self.stdout,
        };

        write!(stream, "{}", args)
    }

    /// Flush the streams.
    pub fn flush(&mut self) -> io::Result<()> {
        self.stdout.flush()?;
        self.stderr.flush()
    }
}

/// A basic selection of colors for printing to the terminal.
///
/// This type provides a simple, built-in palette for colorized printing. You
/// typically won’t need to ever explicitly instantiate it, since it
/// implements `Default` and so does `ColorPrintState`:
///
/// ```
/// #[macro_use] extern crate tcprint;
///
/// use tcprint::{BasicColors, ColorPrintState};
///
/// let mut state = ColorPrintState::<BasicColors>::default();
/// tcprintln!(state, ("Conditions are "), [green: "green"], ("!"));
/// ```
///
/// The listing of fields below shows which colors are available.
///
/// This type implements the `ReportingColors` trait. It returns bold green
/// for `ReportType::Info`, bold yellow for `ReportType::Warning`, and bold
/// red for `ReportType::Error`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BasicColors {
    /// Bold green.
    pub green: ColorSpec,

    /// Bold yellow.
    pub yellow: ColorSpec,

    /// Bold red.
    pub red: ColorSpec,

    /// "Highlight": bold white.
    pub hl: ColorSpec,
}

impl Default for BasicColors {
    fn default() -> Self {
        let mut green = ColorSpec::new();
        green.set_fg(Some(Color::Green)).set_bold(true);

        let mut yellow = ColorSpec::new();
        yellow.set_fg(Some(Color::Yellow)).set_bold(true);

        let mut red = ColorSpec::new();
        red.set_fg(Some(Color::Red)).set_bold(true);

        let mut hl = ColorSpec::new();
        hl.set_bold(true);

        BasicColors {
            green,
            yellow,
            red,
            hl,
        }
    }
}

/// State for colorized printing.
///
/// This structure holds the state needed for colorized printing, namely:
///
/// 1. Handles to colorized versions of the standard output and error streams
/// 2. A palette of colors to use when printing
///
/// Your app should generally create one of these structures early upon
/// startup, and then pass references to it to all modules that need to print
/// to the terminal. Those modules should then use `tcprintln!()`,
/// `tcreport!()`, and related macros to print colorized output to the
/// terminal.
///
/// The type parameter `C` should be a structure containing public fields of
/// type `termcolor::ColorSpec`. These will be accessed inside the macros to
/// simplify the creation of colorized output. The structure `BasicColors`
/// provided by this crate is a simple default that aims to suffice for most
/// purposes. If you want to use `tcreport!()`, the type `C` must implement
/// the `ReportingColors` trait.
///
/// ## Example
///
/// ```
/// #[macro_use] extern crate tcprint;
///
/// use tcprint::{BasicColors, ColorPrintState};
///
/// let mut state = ColorPrintState::<BasicColors>::default();
/// tcreport!(state, warning: "rogue needs food, badly!");
/// ```
///
/// See the crate-level documentation for an example of how to use this
/// structure with a custom color palette.
///
/// ## Technical Note
///
/// The design of this type was the best solution I could come up with for
/// figuring out how to centralize the colored-printing state in a single
/// value, while preserving extensibility and avoiding problems with the
/// borrow-checker. You could imagine using an enumeration of possible colors
/// instead of having the palette type `C` with public fields, but the only
/// way I could see to get that to work would require some heavyweight macros.
#[derive(Default)]
pub struct ColorPrintState<C> {
    streams: PrintStreams,
    colors: C,
}

impl<C> ColorPrintState<C> {
    /// Initialize colorized printing state.
    ///
    /// It is generally preferable to have your color palette type `C`
    /// implement `Default`, and then just create an instance of this type
    /// using `Default::default()`.
    pub fn new(colors: C) -> Self {
        let streams = PrintStreams::default();
        ColorPrintState { streams, colors }
    }

    /// Flush the output streams.
    ///
    /// This method flushes standard output and error.
    pub fn flush(&mut self) -> io::Result<()> {
        self.streams.flush()
    }

    /// Work around borrowck/macro issues.
    #[doc(hidden)]
    pub fn split_into_components_mut<'a>(&'a mut self) -> (&'a mut PrintStreams, &'a C) {
        (&mut self.streams, &self.colors)
    }
}

/// Low-level colorized printing.
///
/// This macro is the generic engine underlying `tcprint!()` and friends.
/// Rather than hardcoding a variant of the `PrintDestination` enumeration to
/// use, it takes the destination type as an additional argument.
#[doc(hidden)]
#[macro_export]
macro_rules! tcanyprint {
    (@clause $cps:expr, $dest:expr, [$color:ident : $($fmt_args:expr),*]) => {{
        let (streams, colors) = $cps.split_into_components_mut();
        let _r = streams.print_color($dest, &colors.$color, format_args!($($fmt_args),*));
    }};

    (@clause $cps:expr, $dest:expr, {$cvar:ident, $cblock:block : $($fmt_args:expr),*}) => {{
        use $crate::ColorSpec;
        let (streams, $cvar) = $cps.split_into_components_mut();
        let c: &ColorSpec = $cblock;
        let _r = streams.print_color($dest, c, format_args!($($fmt_args),*));
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

/// Print to standard output with colorization, without a trailing newline.
///
/// The arguments to this macro are structured as:
///
/// ```ignore
/// tcprint!(state_object, clause1, ...clauseN);
/// ```
///
/// Where `state` is a `ColorPrintState` and each clause takes on one of the
/// following forms:
///
/// - `(format, args...)` to print without applying colorization
/// - `[colorname: format, args...]` to print applying the named color
/// - `{colors_var, block: format, args...}` to print with a color chosen
///   dynamically by evaluating a code block (see example below)
///
/// In all cases the `format, args...` items are passed through the standard
/// Rust [string formatting mechanism](https://doc.rust-lang.org/std/fmt/).
///
/// The `colorname` specifier should refer to a public field of the state
/// object’s "colors" structure. If using the `BasicColors` structure, the
/// available options are: `green`, `yellow`, `red`, and `hl` (highlight).
///
/// ## Examples
///
/// ```
/// # #[macro_use] extern crate tcprint;
/// # use tcprint::{BasicColors, ColorPrintState};
/// # let mut state = ColorPrintState::<BasicColors>::default();
/// let attempt_num = 2;
/// let server = "example.com";
/// tcprint!(state,
///    ("attempting to connect to "),
///    [hl: "{}", server],
///    (" ({}th attempt) ...", attempt_num)
/// );
/// ```
///
/// Note that no spaces are inserted between clauses.
///
/// ```
/// # #[macro_use] extern crate tcprint;
/// # use tcprint::{BasicColors, ColorPrintState};
/// # let mut state = ColorPrintState::<BasicColors>::default();
/// tcprint!(state,
///    ("putting the "),
///    [hl: "fun"],
///    (" in dys"),
///    [yellow: "fun"],
///    ("ctional")
/// );
/// ```
///
/// When using the `{}` specifier, the two parameters are the name of
/// a variable that will be set to your "colors" structure, and a code
/// block that should evaluate to a `&ColorSpec` that will then be used
/// for the printing. This way you can choose colors dynamically based
/// on the values of local variables:
///
/// ```
/// # #[macro_use] extern crate tcprint;
/// # use tcprint::{BasicColors, ColorPrintState};
/// # let mut state = ColorPrintState::<BasicColors>::default();
/// # fn compute_time_left() -> usize { 10 }
/// let seconds_left = compute_time_left();
///
/// tcprint!(state,
///     {colors, {
///         if seconds_left < 5 { &colors.red } else { &colors.hl }
///     }: "{}", seconds_left}, (" seconds left to abort")
/// );
/// ```

#[macro_export]
macro_rules! tcprint {
    ($cps:expr, $($clause:tt),*) => {{
        use $crate::PrintDestination;
        tcanyprint!($cps, PrintDestination::Stdout, $($clause),*)
    }};
}

/// Print to standard error with colorization, without a trailing newline.
///
/// For usage information, see the documentation for `tcprint!()`.
#[macro_export]
macro_rules! etcprint {
    ($cps:expr, $($clause:tt),*) => {{
        use $crate::PrintDestination;
        tcanyprint!($cps, PrintDestination::Stderr, $($clause),*)
    }};
}

/// Print to standard output with colorization and a trailing newline.
///
/// For usage information, see the documentation for `tcprint!()`.
#[macro_export]
macro_rules! tcprintln {
    ($cps:expr, $($clause:tt),*) => {{
        tcprint!($cps, $($clause),*, ("\n"))
    }};
}

/// Print to standard error with colorization and a trailing newline.
///
/// For usage information, see the documentation for `tcprint!()`.
#[macro_export]
macro_rules! etcprintln {
    ($cps:expr, $($clause:tt),*) => {{
        etcprint!($cps, $($clause),*, ("\n"))
    }};
}

/// A helper enumeration of different “report” (log level) types.
///
/// **TODO**: We should play nice with the `log` crate.
///
/// This enumeration is used in the `ReportingColors` trait, for if you want
/// to use the `tcreport!()` macro with a custom color palette type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReportType {
    /// An informational message.
    Info,

    /// A warning.
    Warning,

    /// An error.
    Error,
}

/// Specify colors to be used by the `tcreport!()` macro.
///
/// If you are using a custom color palette for your colorized printing, you
/// must implement this trait on your palette structure if you want to use the
/// `tcreport!()` macro. There is one method to implement, which simply maps
/// between a variant of the `ReportType` enumeration and a
/// `termcolor::ColorSpec` reference.
///
/// ## Example
///
/// ```
/// #[macro_use] extern crate tcprint;
///
/// use std::default::Default;
/// use tcprint::{Color, ColorSpec, ColorPrintState, ReportingColors, ReportType};
///
/// /// In this app, the only "colorization" we use is that sometimes we underline things.
/// #[derive(Clone, Debug, Eq, PartialEq)]
/// struct MyPalette {
///     pub ul: ColorSpec,
/// }
///
/// impl Default for MyPalette {
///     fn default() -> Self {
///         let mut ul = ColorSpec::new();
///         ul.set_underline(true);
///
///         MyPalette { ul }
///     }
/// }
///
/// // Regardless of the report type, the message prefix ("error:", etc.)
/// // will be printed with underlining but no special color.
/// impl ReportingColors for MyPalette {
///     fn get_color_for_report(&self, reptype: ReportType) -> &ColorSpec {
///         &self.ul
///     }
/// }
///
/// fn main() {
///     let mut state = ColorPrintState::<MyPalette>::default();
///     tcreport!(state, info: "all log reports will be prefixed with underlined text");
/// }
/// ```
pub trait ReportingColors {
    /// Get a `termcolor::ColorSpec` to be associated with a report message.
    ///
    /// This color will be used to print the prefix of the message, which will
    /// be something like `warning:`. The main message itself will be printed
    /// with plain colorization.
    fn get_color_for_report(&self, reptype: ReportType) -> &ColorSpec;
}

impl ReportingColors for BasicColors {
    fn get_color_for_report(&self, reptype: ReportType) -> &ColorSpec {
        match reptype {
            ReportType::Info => &self.green,
            ReportType::Warning => &self.yellow,
            ReportType::Error => &self.red,
        }
    }
}

/// Print a colorized log message.
///
/// The syntax of this macro is:
///
/// ```ignore
/// tcreport!(state, level: format, args...);
/// ```
///
/// Where `state` is an expression evaluating to a `ColorPrintState`, `level`
/// is literal text matching one of: `info`, `warning`, or `error`, and
/// `format, args...` are passed through the standard Rust [string formatting
/// mechanism](https://doc.rust-lang.org/std/fmt/).
///
/// ## Example
///
/// ```
/// # #[macro_use] extern crate tcprint;
/// # use tcprint::{BasicColors, ColorPrintState};
/// # let mut state = ColorPrintState::<BasicColors>::default();
/// let pet_type = "puppy";
/// tcreport!(state, warning: "could not locate {}", pet_type);
/// ```
///
/// This will emit the text `warning: could not locate puppy`, where the
/// portion `warning:` appears in bold yellow by default.
///
/// ## Details
///
/// The color palette structure associated with the `ColorPrintState` must
/// implement the `ReportingColors` trait. For the `BasicColors` struct, the
/// `info` level is associated with (bold) green, `warning` with bold yellow,
/// and `error` with bold red.
///
/// Messages of the `info` level are printed to standard output. Messages of
/// `warning` and `error` levels are printed to standard error.
#[macro_export]
macro_rules! tcreport {
    (@inner $cps:expr, $dest:expr, $type:expr, $prefix:expr, $($fmt_args:expr),*) => {{
        {
            use $crate::{PrintDestination, ReportingColors};
            let (streams, colors) = $cps.split_into_components_mut();
            let color = colors.get_color_for_report($type);
            let _r = streams.print_color($dest, color, format_args!($prefix));
        }

        tcprintln!($cps, (" "), ($($fmt_args),*));
    }};

    ($cps:expr, info : $($fmt_args:expr),*) => {{
        use $crate::ReportType;
        tcreport!(@inner $cps, PrintDestination::Stdout, ReportType::Info, "info:", $($fmt_args),*)
    }};

    ($cps:expr, warning : $($fmt_args:expr),*) => {{
        use $crate::ReportType;
        tcreport!(@inner $cps, PrintDestination::Stderr, ReportType::Warning, "warning:", $($fmt_args),*)
    }};

    ($cps:expr, error : $($fmt_args:expr),*) => {{
        use $crate::ReportType;
        tcreport!(@inner $cps, PrintDestination::Stderr, ReportType::Error, "error:", $($fmt_args),*)
    }};
}
