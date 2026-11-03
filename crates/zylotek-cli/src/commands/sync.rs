use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use clap::Subcommand;
use zylotek_application::woods::{
    create_wood::{CreateWood, CreateWoodCommand},
    handlers::WoodEventHandler,
    import_woods::{ImportReport, ImportWoods},
    update_wood::UpdateWood,
};
use zylotek_domain::woods::{
    events::WoodEvent,
    repositories::WoodRepository,
    services::{WoodCreationService, WoodUpdateService},
};
use zylotek_infrastructure::{
    event_bus::tokio::{TokioEventPublisher, run_event_handler},
    persistence::{build_db, woods::repositories::PostgresWoodRepository},
};
use zylotek_sync::{
    OpenData, RejectedRow, Scrape, ScrapedWood, Syncronizer, merge, parse_open_data,
};

type Error = Box<dyn std::error::Error + Send + Sync>;

/// The open-data export that ships with the repository.
pub const DEFAULT_FILE: &str =
    "toasty/fixtures/dados_abertos_banco_de_dados_de_madeiras_brasileiras_lpfsfb.csv";

#[derive(Subcommand, Debug, PartialEq, Eq, Clone)]
pub enum SyncCommand {
    /// Import the open-data CSV, complete it from the LPF web site, and store the woods
    Run {
        #[arg(short, long)]
        config: Option<String>,

        /// The open-data CSV
        #[arg(short, long, default_value = DEFAULT_FILE)]
        file: PathBuf,

        /// The LPF listing to scrape for what the CSV lacks (images, workabilities, ...)
        #[arg(short, long)]
        listing_url: Option<String>,

        /// Import the CSV alone, without visiting the web site
        #[arg(long, conflicts_with = "listing_url")]
        offline: bool,
    },
}

pub fn execute(cmd: &SyncCommand, database_url: &str) -> Result<(), Error> {
    match cmd {
        SyncCommand::Run {
            file,
            listing_url,
            offline,
            ..
        } => cmd_run(file, !offline, listing_url.as_deref(), database_url),
    }
}

fn cmd_run(
    file: &Path,
    use_web: bool,
    listing_url: Option<&str>,
    database_url: &str,
) -> Result<(), Error> {
    let OpenData { woods, rejected } = read_file(file)?;
    let read = woods.len() + rejected.len();

    let (woods, scrape_failures) = if use_web {
        let scrape = scrape(listing_url)?;
        let scraped = scrape.woods.len();
        let merged = merge(woods, scrape.woods);
        println!(
            "Scraped {scraped} species pages from the LPF web site ({} failed)",
            scrape.failures.len()
        );
        println!(
            "Completed {} species from their web pages; {} more exist only on the web",
            merged.enriched, merged.web_only
        );
        (merged.woods, scrape.failures)
    } else {
        (woods, Vec::new())
    };

    let report = save(woods, database_url)?;

    println!("Read {read} rows from {}", file.display());
    println!(
        "Stored {} new woods in {database_url}; updated {}, left {} as they were, skipped {} further lots of a species already read",
        report.created,
        report.updated,
        report.unchanged,
        report.duplicates.len()
    );
    for row in &rejected {
        eprintln!(
            "rejected line {} (id {}): {}",
            row.line,
            id_of(row),
            row.reason
        );
    }
    for failure in &scrape_failures {
        eprintln!("web page failed {}: {}", failure.url, failure.reason);
    }
    for failure in &report.failed {
        eprintln!("failed {}: {}", failure.scientific_name, failure.error);
    }

    match rejected.len() + scrape_failures.len() + report.failed.len() {
        0 => Ok(()),
        problems => Err(format!("{problems} rows or pages could not be imported").into()),
    }
}

fn scrape(listing_url: Option<&str>) -> Result<Scrape, Error> {
    let synchronizer = match listing_url {
        Some(url) => Syncronizer::new().with_listing_url(url),
        None => Syncronizer::new(),
    };

    synchronizer.build().run()
}

fn id_of(row: &RejectedRow) -> &str {
    row.id.as_deref().unwrap_or("?")
}

fn read_file(file: &Path) -> Result<OpenData, Error> {
    let bytes = std::fs::read(file).map_err(|e| format!("cannot read {}: {e}", file.display()))?;

    Ok(parse_open_data(&bytes)?)
}

/// Creates or updates the woods through the same use cases the API uses, so events fire and
/// the scientific-name rule holds.
fn save(woods: Vec<ScrapedWood>, database_url: &str) -> Result<ImportReport, Error> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    runtime.block_on(async {
        let repository: Arc<dyn WoodRepository> =
            Arc::new(PostgresWoodRepository::new(build_db(database_url).await?));
        let publisher = Arc::new(TokioEventPublisher::<WoodEvent>::new(1024));
        let handler = tokio::spawn(run_event_handler(
            publisher.subscribe(),
            WoodEventHandler::new(),
        ));
        let import = ImportWoods::new(
            CreateWood::new(
                publisher.clone(),
                repository.clone(),
                WoodCreationService::new(repository.clone()),
            ),
            UpdateWood::new(
                publisher.clone(),
                repository.clone(),
                WoodUpdateService::new(repository.clone()),
            ),
            repository,
        );

        let report = import
            .execute(woods.into_iter().map(|wood| CreateWoodCommand {
                name: wood.name.to_string(),
                scientific_name: wood.scientific_name.to_string(),
                details: wood.details,
            }))
            .await;

        // Closing the bus lets the handler finish the events still queued.
        drop(import);
        drop(publisher);
        let _ = handler.await;

        Ok::<_, Error>(report)
    })
}
