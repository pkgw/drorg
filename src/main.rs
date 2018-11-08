// Copyright 2018 Peter Williams <peter@newton.cx>
// Licensed under the MIT License.

//! The main CLI driver logic.

#[macro_use] extern crate failure;
extern crate google_drive3;
#[macro_use] extern crate structopt;

use failure::{Error, Fail};
use std::process;
use structopt::StructOpt;


#[derive(Debug, StructOpt)]
pub struct DriverCloseOptions {
    #[structopt(help = "A thingie")]
    what: String,
}

impl DriverCloseOptions {
    fn cli(self) -> Result<i32, Error> {
        Ok(0)
    }
}


#[derive(Debug, StructOpt)]
#[structopt(name = "driver", about = "Deal with Google Drive.")]
pub enum DriverCli {
    #[structopt(name = "close")]
    /// Close a document or something
    Close(DriverCloseOptions),
}

impl DriverCli {
    fn cli(self) -> Result<i32, Error> {
        match self {
            DriverCli::Close(opts) => opts.cli(),
        }
    }
}


fn main() {
    let program = DriverCli::from_args();

    process::exit(match program.cli() {
        Ok(code) => code,

        Err(e) => {
            eprintln!("fatal error in driver");
            for cause in e.iter_chain() {
                eprintln!("  caused by: {}", cause);
            }
            1
        },
    });
}
