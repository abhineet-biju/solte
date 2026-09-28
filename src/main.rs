use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use solte::{
    app::App,
    config::{Config, project_root},
    demo, network, runtime, ui, wallet,
};

#[derive(Parser)]
#[command(
    version,
    about = "A project-aware Solana development wallet for your terminal"
)]
struct Args {
    #[arg(short, long, default_value = ".")]
    project: PathBuf,
    #[arg(long, help = "Use a saved RPC profile by name")]
    profile: Option<String>,
    #[arg(long, help = "Show cached history without making network requests")]
    offline: bool,
    #[arg(
        long,
        help = "Explore the interface using clearly labeled offline fixtures"
    )]
    demo: bool,
    #[arg(long, help = "Disable panel transitions")]
    reduced_motion: bool,
    #[arg(long, help = "Check the selected RPC without opening the TUI")]
    check: bool,
    #[arg(
        long,
        help = "Render the current interface to an SVG file without network access"
    )]
    snapshot: Option<PathBuf>,
    #[arg(long, default_value_t = 160, value_parser = clap::value_parser!(u16).range(80..=300))]
    width: u16,
    #[arg(long, default_value_t = 48, value_parser = clap::value_parser!(u16).range(24..=120))]
    height: u16,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let root = project_root(&args.project)?;
    let mut config = if args.demo {
        Config::default()
    } else {
        Config::load(&root)?
    };
    if let Some(name) = &args.profile {
        config.selected_profile = config
            .profiles
            .iter()
            .position(|p| p.name.eq_ignore_ascii_case(name))
            .context("RPC profile not found")?;
    }
    if args.reduced_motion {
        config.reduced_motion = true;
    }
    if args.check {
        if args.offline || args.demo {
            anyhow::bail!("--check requires network access; omit --offline and --demo");
        }
        let profile = &config.profiles[config.selected_profile];
        let rpc = network::client(profile);
        let (genesis, epoch) = tokio::try_join!(rpc.get_genesis_hash(), rpc.get_epoch_info())
            .map_err(|e| anyhow::anyhow!("{}", network::safe_error(e, profile)))?;
        println!(
            "{}\nEndpoint: {}\nGenesis: {}\nSlot: {}\nBlock height: {}\nEpoch: {}",
            network::cluster_name(&genesis.to_string(), profile),
            profile.display_endpoint(),
            genesis,
            epoch.absolute_slot,
            epoch.block_height,
            epoch.epoch
        );
        return Ok(());
    }
    let (wallets, warnings) = if args.demo {
        (vec![], vec![])
    } else {
        wallet::discover(&root, &config)
    };
    let mut app = App::new(root, config, wallets);
    for warning in warnings {
        app.log("WARN", warning);
    }
    if args.demo {
        demo::populate(&mut app);
    }
    if let Some(path) = args.snapshot {
        ui::snapshot(&app, &path, args.width, args.height)?;
        println!("Saved {}", path.display());
        return Ok(());
    }
    runtime::run(app, args.offline).await
}
