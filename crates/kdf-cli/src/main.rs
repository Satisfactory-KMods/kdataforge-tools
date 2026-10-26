use clap::{Parser, Subcommand, ValueEnum};
use kdf_validation::{
    lint_path, preview_binding, LintOptions, LintResult, LintSettings, OutputFormat, Profile,
    SchemaRegistry, Severity,
};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "kdf", version, about = "KDataForge pack validator")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Lint(LintArgs),
    /// Explain the exact evidence used by lint diagnostics (read-only).
    Why {
        #[command(flatten)]
        lint: LintArgs,
        #[arg(long)]
        rule: Option<String>,
        /// One-based start line of the diagnostic.
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
        line: Option<u32>,
    },
    PreviewDatapack {
        path: PathBuf,
        candidate: PathBuf,
        #[arg(long)]
        user_settings: Option<PathBuf>,
        #[arg(long = "schema-dir")]
        schema_dirs: Vec<PathBuf>,
        #[arg(long)]
        allow_schema_override: bool,
        #[arg(long)]
        allow_unknown_types: bool,
        #[arg(long, default_value = "runtime")]
        profile: ProfileArg,
        #[arg(long, default_value = "json")]
        format: PreviewFormat,
    },
    Schemas {
        #[command(subcommand)]
        action: SchemaAction,
        #[arg(long = "schema-dir", global = true)]
        schema_dirs: Vec<PathBuf>,
        #[arg(long, global = true)]
        allow_schema_override: bool,
    },
}

#[derive(clap::Args)]
struct LintArgs {
    path: PathBuf,
    #[arg(long = "schema-dir")]
    schema_dirs: Vec<PathBuf>,
    #[arg(long)]
    format: Option<Format>,
    #[arg(long)]
    user_settings: Option<PathBuf>,
    #[arg(long)]
    warnings_as_errors: bool,
    #[arg(long)]
    allow_unknown_types: bool,
    #[arg(long)]
    allow_schema_override: bool,
    #[arg(long)]
    datapack: Option<PathBuf>,
    #[arg(long, default_value = "runtime")]
    profile: ProfileArg,
    #[arg(long)]
    no_color: bool,
}

#[derive(Subcommand)]
enum SchemaAction {
    List,
    Check,
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Text,
    Json,
    Github,
    Sarif,
}

impl From<OutputFormat> for Format {
    fn from(value: OutputFormat) -> Self {
        match value {
            OutputFormat::Text => Self::Text,
            OutputFormat::Json => Self::Json,
            OutputFormat::Github => Self::Github,
            OutputFormat::Sarif => Self::Sarif,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum PreviewFormat {
    Json,
}

#[derive(Clone, Copy, ValueEnum)]
enum ProfileArg {
    Runtime,
    Legacy,
}

fn main() {
    let exit = std::panic::catch_unwind(run).unwrap_or_else(|panic| {
        let message = panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&str>().copied())
            .unwrap_or("unknown panic");
        eprintln!("internal error: {message}");
        3
    });
    std::process::exit(exit);
}

fn run() -> i32 {
    let args = Args::parse();
    match args.command {
        Command::Schemas {
            action,
            schema_dirs,
            allow_schema_override,
        } => {
            let options = LintOptions {
                schema_dirs,
                allow_schema_override,
                ..Default::default()
            };
            match SchemaRegistry::new(&options) {
                Ok(registry) => {
                    match action {
                        SchemaAction::List => {
                            for kind in registry.kinds() {
                                println!("{kind}");
                            }
                        }
                        SchemaAction::Check => {
                            println!("schemas valid: {} root type(s)", registry.kinds().len())
                        }
                    }
                    0
                }
                Err(error) => {
                    eprintln!("schema error: {error}");
                    2
                }
            }
        }
        Command::Lint(lint) => run_lint(lint, None),
        Command::Why { lint, rule, line } => run_lint(lint, Some((rule, line))),
        Command::PreviewDatapack {
            path,
            candidate,
            user_settings,
            schema_dirs,
            allow_schema_override,
            allow_unknown_types,
            profile,
            format: PreviewFormat::Json,
        } => {
            let settings = match LintSettings::load(&path, user_settings.as_deref()) {
                Ok(settings) => settings,
                Err(error) => {
                    eprintln!("settings error: {error}");
                    return 2;
                }
            };
            let profile = match profile {
                ProfileArg::Runtime => Profile::Runtime,
                ProfileArg::Legacy => Profile::Legacy,
            };
            let mut options = settings.lint_options(profile);
            options.schema_dirs.extend(schema_dirs);
            if allow_unknown_types {
                options.allow_unknown_types = true;
            }
            if allow_schema_override {
                options.allow_schema_override = true;
            }
            match preview_binding(&path, &[], &candidate, &options) {
                Ok(preview) => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&preview).expect("serialize binding preview")
                    );
                    0
                }
                Err(error) => {
                    eprintln!("preview error: {error}");
                    2
                }
            }
        }
    }
}

fn run_lint(args: LintArgs, why: Option<(Option<String>, Option<u32>)>) -> i32 {
    let LintArgs {
        path,
        schema_dirs,
        format,
        user_settings,
        warnings_as_errors,
        allow_unknown_types,
        allow_schema_override,
        datapack,
        profile,
        no_color: _,
    } = args;
    if why.is_some() && matches!(format, Some(Format::Github | Format::Sarif)) {
        eprintln!("why supports --format json or text");
        return 2;
    }

    let settings = match LintSettings::load(&path, user_settings.as_deref()) {
        Ok(settings) => settings,
        Err(error) => {
            eprintln!("settings error: {error}");
            return 2;
        }
    };
    let profile = match profile {
        ProfileArg::Runtime => Profile::Runtime,
        ProfileArg::Legacy => Profile::Legacy,
    };
    let mut options = settings.lint_options(profile);
    options.schema_dirs.extend(schema_dirs);
    if allow_unknown_types {
        options.allow_unknown_types = true;
    }
    if allow_schema_override {
        options.allow_schema_override = true;
    }
    options.datapack_path = datapack;
    let result = match lint_path(&path, &options) {
        Ok(result) => result,
        Err(error) => {
            eprintln!("schema error: {error}");
            return 2;
        }
    };
    if let Some((rule, line)) = why {
        let report = kdf_cli::why_report(&result, rule.as_deref(), line);
        if matches!(format, Some(Format::Text)) {
            for entry in report["explanations"].as_array().unwrap() {
                println!(
                    "{}: {}\nEvidence: {}\nAction: {}",
                    entry["ruleId"].as_str().unwrap(),
                    entry["message"].as_str().unwrap(),
                    entry["evidence"]["reason"].as_str().unwrap(),
                    entry["action"].as_str().unwrap()
                );
            }
        } else {
            println!("{report}");
        }
        return kdf_cli::lint_exit_code(&result, warnings_as_errors || settings.warnings_as_errors);
    }
    match format.unwrap_or_else(|| settings.output_format.into()) {
        Format::Text => print_text(&result),
        Format::Json => println!("{}", kdf_cli::json_report(&result)),
        Format::Github => print_github(&result),
        Format::Sarif => println!("{}", kdf_cli::sarif_report(&result)),
    }
    kdf_cli::lint_exit_code(&result, warnings_as_errors || settings.warnings_as_errors)
}

fn print_text(result: &LintResult) {
    for item in &result.diagnostics {
        let location = item
            .file
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<configuration>".into());
        let suffix = match (item.line, item.column) {
            (Some(line), Some(column)) => format!(":{line}:{column}"),
            (Some(line), None) => format!(":{line}"),
            _ => String::new(),
        };
        let document = item
            .document
            .map(|index| format!("[{index}]"))
            .unwrap_or_default();
        eprintln!(
            "{}: {location}{suffix}{document}: {} ({}, {})",
            severity(item.severity),
            item.message,
            item.code,
            item.yaml_path
        );
    }
    if result.ok {
        println!(
            "lint passed: {} pack(s), {} YAML document(s)",
            result.packs, result.documents
        );
    } else {
        eprintln!(
            "lint failed: {} error(s), {} warning(s)",
            result.errors, result.warnings
        );
    }
}

fn print_github(result: &LintResult) {
    for item in &result.diagnostics {
        let mut properties = Vec::new();
        if let Some(file) = &item.file {
            properties.push(format!(
                "file={}",
                github_property(&file.display().to_string())
            ));
        }
        if let Some(line) = item.line {
            properties.push(format!("line={line}"));
        }
        if let Some(column) = item.column {
            properties.push(format!("col={column}"));
        }
        let message = github_message(&format!(
            "{} [{} {}]",
            item.message, item.code, item.yaml_path
        ));
        println!(
            "::{} {}::{message}",
            if item.advisory.is_some() || item.severity == Severity::Information {
                "notice"
            } else {
                severity(item.severity)
            },
            properties.join(",")
        );
    }
    println!(
        "Validated {} pack(s) and {} YAML document(s).",
        result.packs, result.documents
    );
}

fn github_message(value: &str) -> String {
    value
        .replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

fn github_property(value: &str) -> String {
    github_message(value)
        .replace(':', "%3A")
        .replace(',', "%2C")
}

fn severity(value: Severity) -> &'static str {
    match value {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Information => "information",
    }
}
