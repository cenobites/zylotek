use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use clap::{Subcommand, ValueEnum};
use zylotek_application::woods::{
    create_wood::{CreateWood, CreateWoodCommand},
    handlers::WoodEventHandler,
    import_woods::{ImportReport, ImportWoods},
};
use zylotek_domain::woods::{
    events::WoodEvent, repositories::WoodRepository, services::WoodCreationService,
};
use zylotek_infrastructure::{
    event_bus::tokio::{TokioEventPublisher, run_event_handler},
    persistence::{build_db, woods::repositories::PostgresWoodRepository},
};
use zylotek_sync::{OpenData, RejectedRow, ScrapedWood, Syncronizer, parse_open_data};

type Error = Box<dyn std::error::Error + Send + Sync>;

/// The open-data export that ships with the repository.
pub const DEFAULT_FILE: &str =
    "toasty/fixtures/dados_abertos_banco_de_dados_de_madeiras_brasileiras_lpfsfb.csv";

#[derive(ValueEnum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Source {
    /// Import the open-data CSV
    File,
    /// Scrape the LPF web site
    Web,
}

#[derive(Subcommand, Debug, PartialEq, Eq, Clone)]
pub enum SyncCommand {
    /// Load woods from a source and store them in the database
    Run {
        #[arg(short, long)]
        config: Option<String>,

        /// Where the woods come from
        #[arg(short, long, value_enum, default_value_t = Source::File)]
        source: Source,

        /// The CSV to import when the source is `file`
        #[arg(short, long, default_value = DEFAULT_FILE)]
        file: PathBuf,
    },
}

pub fn execute(cmd: &SyncCommand, database_url: &str) -> Result<(), Error> {
    match cmd {
        SyncCommand::Run { source, file, .. } => cmd_run(*source, file, database_url),
    }
}

fn cmd_run(source: Source, file: &Path, database_url: &str) -> Result<(), Error> {
    let (woods, rejected, origin) = match source {
        Source::File => {
            let OpenData { woods, rejected } = read_file(file)?;
            (woods, rejected, file.display().to_string())
        }
        Source::Web => (
            Syncronizer::new().build().run()?,
            Vec::new(),
            "the LPF web site".to_owned(),
        ),
    };

    let read = woods.len() + rejected.len();
    let report = save(woods, database_url)?;

    println!("Read {read} rows from {origin}");
    println!(
        "Stored {} new woods in {database_url}; skipped {} whose scientific name is already registered",
        report.created,
        report.already_registered.len()
    );
    for row in &rejected {
        eprintln!(
            "rejected line {} (id {}): {}",
            row.line,
            id_of(row),
            row.reason
        );
    }
    for failure in &report.failed {
        eprintln!("failed {}: {}", failure.scientific_name, failure.error);
    }

    match rejected.len() + report.failed.len() {
        0 => Ok(()),
        problems => Err(format!("{problems} rows could not be imported").into()),
    }
}

fn id_of(row: &RejectedRow) -> &str {
    row.id.as_deref().unwrap_or("?")
}

fn read_file(file: &Path) -> Result<OpenData, Error> {
    let bytes = std::fs::read(file).map_err(|e| format!("cannot read {}: {e}", file.display()))?;

    Ok(parse_open_data(&bytes)?)
}

/// Stores the woods through the same `CreateWood` use case the API uses, so events fire and
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
        let import = ImportWoods::new(CreateWood::new(
            publisher.clone(),
            repository.clone(),
            WoodCreationService::new(repository),
        ));

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
