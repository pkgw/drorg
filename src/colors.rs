// Copyright 2018 Peter Williams <peter@newton.cx>
// Licensed under the MIT License.

//! The color palette for the command line interface.

use tcprint::{Color, ColorSpec, ReportingColors, ReportType};


/// The CLI color palette.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Colors {
    /// Bold green.
    pub green: ColorSpec,

    /// Bold yellow.
    pub yellow: ColorSpec,

    /// Bold red.
    pub red: ColorSpec,

    /// "Highlight": bold white.
    pub hl: ColorSpec,

    /// A totally plain color; useful in some conditional-coloring scenarios.
    pub plain: ColorSpec,

    /// The color for a "%NN" document short-hand in a listing;
    /// defaults to red.
    pub percent_tag: ColorSpec,

    /// The color for a folder; defaults to bold blue.
    pub folder: ColorSpec,
}


impl Default for Colors {
    fn default() -> Self {
        let mut green = ColorSpec::new();
        green.set_fg(Some(Color::Green)).set_bold(true);

        let mut yellow = ColorSpec::new();
        yellow.set_fg(Some(Color::Yellow)).set_bold(true);

        let mut red = ColorSpec::new();
        red.set_fg(Some(Color::Red)).set_bold(true);

        let mut hl = ColorSpec::new();
        hl.set_bold(true);

        let plain = ColorSpec::new();

        let mut percent_tag = ColorSpec::new();
        percent_tag.set_fg(Some(Color::Red));

        let mut folder = ColorSpec::new();
        folder.set_fg(Some(Color::Blue)).set_bold(true);

        Colors {
            green,
            yellow,
            red,
            hl,
            plain,
            percent_tag,
            folder,
        }
    }
}

impl ReportingColors for Colors {
    fn get_color_for_report(&self, reptype: ReportType) -> &ColorSpec {
        match reptype {
            ReportType::Info => &self.green,
            ReportType::Warning => &self.yellow,
            ReportType::Error => &self.red,
        }
    }
}
