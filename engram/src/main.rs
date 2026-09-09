use clap::{Parser, Subcommand};
use engram::compile::{
    get_context_with, search_code, search_symbols, GetContextOpts, DEFAULT_BUDGET,
};
use engram::doctor::{run_doctor, run_status};
use engram::error::Error;
use engram::index::{index_repo_with, IndexOpts, IndexStats};
use engram::init::{run_init, write_harness, write_skill};
use engram::render::render_digest;
use engram::root::{env_root, find_repo_root};
use std::path::PathBuf;
use std::process;

const SEARCH_LIMIT: usize = 20;

#[derive(Parser)]
#[command(name = "engram", version, about = "Local extractive code intelligence")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create `.engram/`, empty DB, and ignore files (does not index)
    Init {
        /// Write project MCP snippet: grok|copilot|claude|cursor|all
        #[arg(long)]
        harness: Option<String>,
        /// Write get_context skill under `.grok/skills/engram/`
        #[arg(long)]
        skill: bool,
        /// Create `AGENTS.md` when writing skill (append if it already exists)
        #[arg(long)]
        write_agents: bool,
    },
    /// Incremental index (`--force` rebuilds)
    Index {
        #[arg(long)]
        force: bool,
        /// Nested project directory to index into the workspace DB
        #[arg(long, value_name = "DIR")]
        path: Option<PathBuf>,
    },
    /// DB path, schema, counts, last index, stale sample
    Status,
    /// Compile an extractive context package
    GetContext {
        query: String,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = DEFAULT_BUDGET)]
        budget: u32,
        /// Attach optional MemPalace drawers when available
        #[arg(long)]
        palace: bool,
    },
    /// Debug: symbol lookup
    SearchSymbols { name: String },
    /// Debug: FTS lookup
    SearchCode { query: String },
    /// MCP stdio server
    Mcp,
    /// Check binary, grammars, DB, ignore files, harness config
    Doctor,
}

fn main() {
    match Cli::try_parse() {
        Ok(cli) => {
            if let Err(err) = dispatch(cli) {
                eprintln!("{err}");
                process::exit(err.exit_code());
            }
        }
        Err(e) => {
            let _ = e.print();
            process::exit(if e.use_stderr() { 1 } else { 0 });
        }
    }
}

fn dispatch(cli: Cli) -> Result<(), Error> {
    match cli.command {
        Commands::Init {
            harness,
            skill,
            write_agents,
        } => {
            let cwd = current_dir()?;
            let root = run_init(&cwd)?;
            if let Some(ref id) = harness {
                write_harness(&root, id)?;
            }
            if skill {
                let also_claude = matches!(harness.as_deref(), Some("claude") | Some("all"));
                write_skill(&root, also_claude, write_agents)?;
            }
            println!("initialized {}", root.display());
            Ok(())
        }
        Commands::Index { force, path } => {
            let root = require_root()?;
            let opts = match path {
                None => IndexOpts::default(),
                Some(p) => {
                    let abs = if p.is_absolute() {
                        p
                    } else {
                        current_dir()?.join(p)
                    };
                    let resolved = engram::root::resolve_index_walk(&root, &abs)?;
                    IndexOpts {
                        walk: resolved.prefix.as_ref().map(|_| resolved.dir),
                        ..Default::default()
                    }
                }
            };
            let stats = index_repo_with(&root, force, opts)?;
            print_index_stats(&stats);
            Ok(())
        }
        Commands::Status => {
            print!("{}", run_status(&require_root()?)?);
            Ok(())
        }
        Commands::GetContext {
            query,
            json,
            budget,
            palace,
        } => {
            let include_palace = if palace { Some(true) } else { None };
            let pkg = get_context_with(
                &require_root()?,
                &query,
                budget,
                GetContextOpts {
                    include_palace,
                    palace_search: None,
                },
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&pkg).expect("json"));
            } else {
                print!("{}", render_digest(&pkg));
            }
            Ok(())
        }
        Commands::SearchSymbols { name } => {
            for h in search_symbols(&require_root()?, &name, SEARCH_LIMIT)? {
                println!(
                    "{}:{}-{} {} {}",
                    h.path,
                    h.start_line,
                    h.end_line,
                    h.kind.as_str(),
                    h.name
                );
            }
            Ok(())
        }
        Commands::SearchCode { query } => {
            for h in search_code(&require_root()?, &query, SEARCH_LIMIT)? {
                println!("{}\t{}", h.path, h.rank);
            }
            Ok(())
        }
        Commands::Mcp => engram::mcp::run(),
        Commands::Doctor => {
            print!("{}", run_doctor(&require_root()?)?);
            Ok(())
        }
    }
}

fn current_dir() -> Result<PathBuf, Error> {
    std::env::current_dir().map_err(Error::from)
}

fn require_root() -> Result<PathBuf, Error> {
    find_repo_root(&current_dir()?, env_root().as_deref())
}

fn print_index_stats(stats: &IndexStats) {
    let skipped = stats.skipped_secret + stats.skipped_large + stats.skipped_ignore;
    println!("files: {}", stats.files);
    println!("symbols: {}", stats.symbols);
    println!("edges: {}", stats.edges);
    println!("skipped: {}", skipped);
    println!("errors: {}", stats.errors);
    println!("commits: {}", stats.commits);
    println!("git: {}", stats.git.as_str());
}
