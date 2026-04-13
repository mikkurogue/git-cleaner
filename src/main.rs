use anyhow::{anyhow, Result};
use ariadne::{Color, Label, Report, ReportKind, Source};
use chrono::{DateTime, Duration, Utc};
use clap::Parser;
use colored::*;
use dialoguer::{theme::ColorfulTheme, MultiSelect};
use std::collections::BTreeMap;
use std::ops::Range;
use std::process::Command;

#[derive(Parser, Debug)]
#[command(name = "git-prune-branches")]
#[command(about = "Delete stale remote git branches by author + age")]
struct Args {
    #[arg(
        long,
        value_delimiter = ',',
        help = "Author email(s) to match, comma-separated"
    )]
    author: Vec<String>,

    #[arg(long, help = "Date (YYYY-MM-DD) or relative (e.g. 30d, 6m, 1y)")]
    before: String,

    #[arg(long, default_value = "origin")]
    remote: String,

    #[arg(long, default_value_t = false)]
    dry: bool,

    #[arg(long, default_value_t = false)]
    yes: bool,

    #[arg(long, value_delimiter = ',', default_value = "main,master,develop")]
    protected: Vec<String>,
}

struct BranchInfo {
    name: String,
    author: String,
    date: DateTime<Utc>,
}

fn parse_date(input: &str) -> Result<DateTime<Utc>> {
    // Absolute date
    if let Ok(dt) = DateTime::parse_from_rfc3339(&(input.to_string() + "T00:00:00Z")) {
        return Ok(dt.with_timezone(&Utc));
    }

    // Relative formats: 30d, 6m, 1y
    let now = Utc::now();

    let (num, unit) = input.split_at(input.len() - 1);
    let value: i64 = num.parse()?;

    let duration = match unit {
        "d" => Duration::days(value),
        "m" => Duration::days(value * 30),
        "y" => Duration::days(value * 365),
        _ => return Err(anyhow!("Invalid date format")),
    };

    Ok(now - duration)
}

/// Print one ariadne report per author, where each branch is a line in
/// a synthetic source and a label spans the branch name with the commit date.
fn print_reports(
    branches: &[BranchInfo],
    author_colors: &BTreeMap<String, Color>,
    cutoff: &DateTime<Utc>,
) -> Result<()> {
    // Group branches by author
    let mut by_author: BTreeMap<&str, Vec<&BranchInfo>> = BTreeMap::new();
    for b in branches {
        by_author.entry(&b.author).or_default().push(b);
    }

    for (author, author_branches) in &by_author {
        let color = author_colors
            .get(*author)
            .copied()
            .unwrap_or(Color::Primary);

        // Build synthetic source: one branch name per line
        let mut source = String::new();
        let mut labels: Vec<Label<Range<usize>>> = Vec::new();

        for branch in author_branches {
                    let start = source.len();
                    source.push_str(&branch.name);
                    let end = source.len();

                    labels.push(
                        Label::new(start..end)
                            .with_message(format!("last commit {}", branch.date.format("%Y-%m-%d")))
                            .with_color(color),
                    );

                    source.push('\n');
                };
        
        let report = Report::build(ReportKind::Custom("Stale", color), 0..0)
            .with_message(format!(
                "{} — {} stale branches (before {})",
                author,
                author_branches.len(),
                cutoff.format("%Y-%m-%d"),
            ))
            .with_labels(labels)
            .finish();

        report.print(Source::from(&source))?;
        println!();
    }

    Ok(())
}

fn main() -> Result<()> {
    let args = Args::parse();
    let repo = gix::open(".")?;

    let cutoff = parse_date(&args.before)?;

    println!("{}", "Scanning branches...".bold());
    println!("Author: {}", args.author.join(", "));
    println!("Before:    {}", cutoff.format("%Y-%m-%d"));
    println!("Remote:    {}", args.remote);
    println!();

    let mut branches: Vec<BranchInfo> = Vec::new();
    let prefix = format!("refs/remotes/{}/", args.remote);

    for reference in repo.references()?.all()? {
        let mut reference = reference.map_err(|e| anyhow!("{e}"))?;
        let name = reference.name().as_bstr().to_string();

        if !name.starts_with(&prefix) {
            continue;
        }

        if name.ends_with("/HEAD") {
            continue;
        }

        let branch = name.strip_prefix(&prefix).unwrap().to_string();

        if args.protected.contains(&branch) {
            continue;
        }

        let commit = reference.peel_to_commit()?;
        let author = commit.author()?;

        let email = author.email.to_string();
        let ts = author.seconds();

        let commit_dt =
            DateTime::<Utc>::from_timestamp(ts, 0).ok_or_else(|| anyhow!("Invalid timestamp"))?;

        // Filter by author if specified, otherwise match all
        let author_match = args.author.is_empty() || args.author.contains(&email);

        if author_match && commit_dt < cutoff {
            branches.push(BranchInfo {
                name: branch,
                author: email,
                date: commit_dt,
            });
        }
    }

    if branches.is_empty() {
        println!("{}", "No stale branches found.".green());
        return Ok(());
    }

    // Sort by author, then by date
    branches.sort_by(|a, b| a.author.cmp(&b.author).then(a.date.cmp(&b.date)));

    const ONEDARK_GREEN: Color = Color::Rgb(0x98, 0xc3, 0x79);

    let mut author_colors: BTreeMap<String, Color> = BTreeMap::new();
    for branch in &branches {
        author_colors
            .entry(branch.author.clone())
            .or_insert(ONEDARK_GREEN);
    }

    // --- Ariadne reports: one per author (im retarded so its not working for multi author yet) ---
    print_reports(&branches, &author_colors, &cutoff)?;

    if args.dry {
        println!("{}", "Dry run - no changes made.".blue());
        return Ok(());
    }

    let items: Vec<String> = branches
        .iter()
        .map(|b| format!("{} ({}, {})", b.name, b.author, b.date.format("%Y-%m-%d")))
        .collect();

    // Ignore prompt and just go if --yes is provided
    if args.yes {
        for branch in &branches {
            println!("Deleting {}", branch.name.red());
            Command::new("git")
                .args(["push", &args.remote, "--delete", &branch.name])
                .status()?;
        }
    } else {
        let defaults: Vec<bool> = vec![true; items.len()];

        let selections = MultiSelect::with_theme(&ColorfulTheme::default())
            .with_prompt("Select branches to delete (Space to toggle, Enter to confirm)")
            .items(&items)
            .defaults(&defaults)
            .interact_opt()?;

        match selections {
            None => {
                println!("Aborted.");
                return Ok(());
            }
            Some(indices) if indices.is_empty() => {
                println!("No branches selected.");
                return Ok(());
            }
            Some(indices) => {
                println!();
                for i in &indices {
                    let branch = &branches[*i];
                    println!("Deleting {}", branch.name.red());
                    Command::new("git")
                        .args(["push", &args.remote, "--delete", &branch.name])
                        .status()?;
                }
            }
        }
    }

    println!("{}", "\nDone!".green().bold());
    Ok(())
}
