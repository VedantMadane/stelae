pub mod db_data;

use crate::archive_testtools::{self, config::ArchiveType, utils};
use actix_http::Request;
use actix_service::Service;
use actix_web::{
    Error, dev::ServiceResponse, rt::time, test::{self}
};
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::Once;
use stelae::db::{self, DatabaseConnection};
use std::time::Duration;
use stelae::server::api::state::Global;
use tempfile::Builder;
static INIT: Once = Once::new();

use actix_http::body::MessageBody;

use stelae::server::app;
use stelae::stelae::archive::Archive;

pub const BASIC_MODULE_NAME: &str = "basic";

pub fn blob_to_string(blob: Vec<u8>) -> String {
    core::str::from_utf8(blob.as_slice()).unwrap().into()
}

// TODO: consider adding abort! test macro,
// which aborts the current test.
// then we can manually inspect the state of the test environment

// to manually inspect state of test environment at present,
// we use anyhow::bail!() which aborts the entire test suite.

#[derive(Debug, Clone)]
pub struct TestAppState {
    pub archive: Archive,
    pub db: DatabaseConnection,
}

impl Global for TestAppState {
    fn archive(&self) -> &Archive {
        &self.archive
    }
    fn db(&self) -> &db::DatabaseConnection {
        &self.db
    }
}

pub async fn initialize_app(
    archive_path: &Path,
) -> impl Service<Request, Response = ServiceResponse<impl MessageBody>, Error = Error> {
    let archive = Archive::parse(archive_path.to_path_buf(), archive_path, false).unwrap();
    let db = match db::init::connect(&archive_path).await {
        Ok(db) => db,
        Err(err) => {
            tracing::error!(
                "error: could not connect to database. Confirm that DATABASE_URL env var is set correctly."
            );
            tracing::error!("Error: {:?}", err);
            panic!()
        }
    };
    let state = TestAppState { archive, db };
    let app = app::init(&state).unwrap();
    test::init_service(app).await
}

/// Like `initialize_app`, but also returns a `DatabaseConnection` whose pool can be
/// explicitly closed (`db.pool.close().await`) before the `TempDir` drops.
/// This is necessary on Windows where SQLite WAL files remain locked until all pool
/// connections are fully closed, preventing `TempDir::drop` from deleting the directory.
pub async fn initialize_app_with_db(
    archive_path: &Path,
) -> (
    impl Service<Request, Response = ServiceResponse<impl MessageBody>, Error = Error>,
    DatabaseConnection,
) {
    let archive = Archive::parse(archive_path.to_path_buf(), archive_path, false).unwrap();
    let db = match db::init::connect(&archive_path).await {
        Ok(db) => db,
        Err(err) => {
            tracing::error!(
                "error: could not connect to database. Confirm that DATABASE_URL env var is set correctly."
            );
            tracing::error!("Error: {:?}", err);
            panic!()
        }
    };
    // Override WAL mode for tests. The DB is empty at this point so the
    // checkpoint is a no-op; SQLite never creates -wal/-shm files, which
    // makes TempDir cleanup reliable on Windows.
    let _ = sqlx::query("PRAGMA journal_mode=DELETE")
        .execute(&db.pool)
        .await;
    let db_handle = db.clone();
    let state = TestAppState { archive, db };
    let app = app::init(&state).unwrap();
    (test::init_service(app).await, db_handle)
}

pub async fn get_db(archive_path: &Path) -> DatabaseConnection {
    match db::init::connect(&archive_path).await {
        Ok(db) => db,
        Err(err) => {
            tracing::error!(
                "error: could not connect to database. Confirm that DATABASE_URL env var is set correctly."
            );
            tracing::error!("Error: {:?}", err);
            panic!()
        }
    }
}

pub fn initialize_archive(archive_type: ArchiveType) -> Result<tempfile::TempDir> {
    match initialize_archive_without_bare(archive_type) {
        Ok(td) => {
            if let Err(err) = utils::make_all_git_repos_bare_recursive(&td) {
                return Err(err);
            }
            Ok(td)
        }
        Err(err) => Err(err),
    }
}

pub fn initialize_archive_without_bare(archive_type: ArchiveType) -> Result<tempfile::TempDir> {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests/fixtures/");

    let td = Builder::new().tempdir_in(&path).unwrap();

    if let Err(err) = archive_testtools::initialize_archive_inner(archive_type, &td) {
        dbg!(&err);
        use std::mem::ManuallyDrop;
        let td = ManuallyDrop::new(td);
        // TODO: better error handling on testing failure
        let error_output_directory = path.clone().join(PathBuf::from("error_output_directory"));
        std::fs::remove_dir_all(&error_output_directory).unwrap();
        std::fs::rename(td.path(), &error_output_directory).expect("Failed to move temp directory");
        eprintln!(
                "{}", format!("Failed to remove '{error_output_directory:?}', please try to remove directory by hand. Original error: {err}")
            );
        return Err(err);
    }
    Ok(td)
}

/// Used to initialize the test environment for git micro-server.
pub fn initialize_git() {
    INIT.call_once(|| {
        let repo_path =
            get_test_archive_path(BASIC_MODULE_NAME).join(PathBuf::from("test/law-html"));
        let heads_path = repo_path.join(PathBuf::from("refs/heads"));
        std::fs::create_dir_all(heads_path).unwrap();
        let tags_path = repo_path.join(PathBuf::from("refs/tags"));
        std::fs::create_dir_all(tags_path).unwrap();
    });
}

pub fn get_test_archive_path(mod_name: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests/fixtures/");
    path.push(mod_name.to_owned() + "/archive");
    path
}
