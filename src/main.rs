use clap::{Parser, Subcommand};
use std::path::PathBuf;
use vex::git;

#[derive(Parser)]
#[command(name = "vex", version, about = "A tiny git-like toy implementation")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Add { paths: Vec<PathBuf> },
    CatFile { oid: String },
    CheckIgnore { paths: Vec<PathBuf> },
    Checkout { name: String },
    Commit { #[arg(short, long)] message: String },
    HashObject { #[arg(short = 'w', long)] write: bool, path: PathBuf },
    Init { path: Option<PathBuf> },
    Log { name: Option<String> },
    LsFiles,
    LsTree { name: String },
    RevParse { name: String },
    Rm { #[arg(long)] cached: bool, paths: Vec<PathBuf> },
    ShowRef { #[arg(long)] head: bool },
    Status,
    Tag { name: String, target: Option<String> },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Init { path } => {
            let path = path.unwrap_or_else(|| std::env::current_dir().unwrap());
            git::cmd_init(&path)?;
        }
        Command::HashObject { write, path } => {
            let repo = git::repo_find(&std::env::current_dir()?)?;
            git::cmd_hash_object(&repo, &path, write)?;
        }
        Command::CatFile { oid } => {
            let repo = git::repo_find(&std::env::current_dir()?)?;
            git::cmd_cat_file(&repo, &oid)?;
        }
        Command::LsTree { name } => {
            let repo = git::repo_find(&std::env::current_dir()?)?;
            git::cmd_ls_tree(&repo, &name)?;
        }
        Command::Add { paths } => {
            let repo = git::repo_find(&std::env::current_dir()?)?;
            git::cmd_add(&repo, &paths)?;
        }
        Command::LsFiles => {
            let repo = git::repo_find(&std::env::current_dir()?)?;
            git::cmd_ls_files(&repo)?;
        }
        Command::Status => {
            let repo = git::repo_find(&std::env::current_dir()?)?;
            git::cmd_status(&repo)?;
        }
        Command::Rm { cached, paths } => {
            let repo = git::repo_find(&std::env::current_dir()?)?;
            git::cmd_rm(&repo, &paths, cached)?;
        }
        Command::Checkout { name } => {
            let repo = git::repo_find(&std::env::current_dir()?)?;
            git::cmd_checkout(&repo, &name)?;
        }
        Command::Commit { message } => {
            let repo = git::repo_find(&std::env::current_dir()?)?;
            git::cmd_commit(&repo, &message)?;
        }
        Command::Log { name } => {
            let repo = git::repo_find(&std::env::current_dir()?)?;
            git::cmd_log(&repo, name.as_deref())?;
        }
        Command::RevParse { name } => {
            let repo = git::repo_find(&std::env::current_dir()?)?;
            git::cmd_rev_parse(&repo, &name)?;
        }
        Command::ShowRef { head } => {
            let repo = git::repo_find(&std::env::current_dir()?)?;
            git::cmd_show_ref(&repo, head)?;
        }
        Command::Tag { name, target } => {
            let repo = git::repo_find(&std::env::current_dir()?)?;
            git::cmd_tag(&repo, &name, target.as_deref())?;
        }
        Command::CheckIgnore { paths } => {
            let repo = git::repo_find(&std::env::current_dir()?)?;
            git::cmd_check_ignore(&repo, &paths)?;
        }
    }
    Ok(())
}
