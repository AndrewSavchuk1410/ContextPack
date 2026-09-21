use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use contextpack_core::{PlanError, collect_plan, load_plan};
use contextpack_render::render_markdown;

#[derive(Debug, Parser)]
#[command(
    name = "contextpack",
    version,
    about = "Deterministic read-only repository evidence collector"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Parse, strictly validate, and print the normalized V1 plan as JSON.
    Validate {
        /// CollectionPlan YAML file.
        plan: PathBuf,
    },
    /// Collect repository evidence and render a V1 Markdown Context Pack.
    Collect {
        /// CollectionPlan YAML file.
        plan: PathBuf,
        /// Markdown output path. Omit to write to stdout.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Optional canonical internal result JSON output path.
        #[arg(long)]
        json: Option<PathBuf>,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(AppError::Plan(error)) => {
            for diagnostic in error.diagnostics {
                let query = diagnostic
                    .query_id
                    .as_deref()
                    .map(|id| format!(" query `{id}`"))
                    .unwrap_or_default();
                eprintln!(
                    "[error] {}{}: {}",
                    diagnostic.code, query, diagnostic.message
                );
            }
            ExitCode::from(2)
        }
        Err(AppError::Io(error)) => {
            eprintln!("[error] io_error: {error}");
            ExitCode::from(1)
        }
        Err(AppError::Json(error)) => {
            eprintln!("[error] internal_error: {error}");
            ExitCode::from(1)
        }
    }
}

fn run(cli: Cli) -> Result<(), AppError> {
    match cli.command {
        Command::Validate { plan } => {
            let plan = load_plan(plan)?;
            let output = serde_json::to_string_pretty(&plan.normalized_json())?;
            println!("{output}");
        }
        Command::Collect { plan, output, json } => {
            let plan = load_plan(plan)?;
            let result = collect_plan(&plan);
            let markdown = render_markdown(&result);
            if let Some(path) = output {
                fs::write(path, markdown)?;
            } else {
                let mut stdout = io::stdout().lock();
                stdout.write_all(markdown.as_bytes())?;
            }
            if let Some(path) = json {
                let bytes = serde_json::to_vec_pretty(&result)?;
                fs::write(path, bytes)?;
            }
        }
    }
    Ok(())
}

enum AppError {
    Plan(PlanError),
    Io(io::Error),
    Json(serde_json::Error),
}

impl From<PlanError> for AppError {
    fn from(value: PlanError) -> Self {
        Self::Plan(value)
    }
}

impl From<io::Error> for AppError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for AppError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}
