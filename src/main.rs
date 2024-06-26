use axum::{
    extract::State,
    routing::get,
    Router,
};
use surrealdb::engine::any::{Any, connect};
use surrealdb::opt::auth::Root;
use surrealdb::{Error as SurrealError, Surreal};
use std::sync::Arc;
use serde_json::Value;
use std::time::Duration;
use tokio::time::sleep;

struct AppState {
    db: Surreal<Any>,
}

#[derive(Debug)]
enum CustomError {
    SurrealError(SurrealError),
    MaxRetriesReached,
}

impl From<SurrealError> for CustomError {
    fn from(err: SurrealError) -> Self {
        CustomError::SurrealError(err)
    }
}

impl std::fmt::Display for CustomError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CustomError::SurrealError(err) => write!(f, "SurrealDB error: {}", err),
            CustomError::MaxRetriesReached => write!(f, "Maximum retry attempts reached"),
        }
    }
}

async fn setup_db() -> Result<Surreal<Any>, CustomError> {
    let max_retries = 16;
    let retry_delay = Duration::from_secs(8);

    for attempt in 1..=max_retries {
        println!("Attempting to connect to database (attempt {}/{})", attempt, max_retries);
        match connect_db().await {
            Ok(db) => {
                println!("Successfully connected to database on attempt {}/{}", attempt, max_retries);
                return Ok(db);
            }
            Err(e) if attempt < max_retries => {
                eprintln!("Failed to connect to database (attempt {}/{}): {}. Retrying in {} seconds...", 
                    attempt, max_retries, e, retry_delay.as_secs());
                sleep(retry_delay).await;
            }
            Err(e) => return Err(CustomError::SurrealError(e)),
        }
    }
    println!("Maximum retry attempts ({}/{}) reached. Giving up.", max_retries, max_retries);
    Err(CustomError::MaxRetriesReached)
}

async fn connect_db() -> Result<Surreal<Any>, SurrealError> {
    let db = connect("ws://localhost:8000").await?;
    db.signin(Root {
        username: "root",
        password: "root",
    })
    .await?;
    db.use_ns("test").use_db("test").await?;
    Ok(db)
}

async fn get_data(State(state): State<Arc<AppState>>) -> Result<String, String> {
    let result: Vec<Value> = state.db.query("SELECT * FROM customer LIMIT 1").await
        .map_err(|e| e.to_string())?
        .take(0)
        .map_err(|e| e.to_string())?;
    
    if result.is_empty() {
        Ok("No data found".to_string())
    } else {
        Ok(serde_json::to_string_pretty(&result[0]).map_err(|e| e.to_string())?)
    }
}

#[tokio::main]
async fn main() {
    println!("==== main ====");
    let db = match setup_db().await {
        Ok(db) => db,
        Err(e) => {
            eprintln!("Failed to setup database after multiple attempts: {}", e);
            return;
        }
    };

    println!("==== 02 ====");
    let state = Arc::new(AppState { db });

    println!("==== 03 ====");
    let app = Router::new()
        .route("/data", get(get_data))
        .with_state(state);

    println!("==== 04 ====");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3456").await.unwrap();
    let addr = listener.local_addr().unwrap();
    println!("Listening on {}", addr);
    axum::serve(listener, app).await.unwrap();
}

