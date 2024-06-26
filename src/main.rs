use axum::{
    extract::{State, Json},
    routing::{get, post},
    Router, 
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{Response, IntoResponse},
    body::Body,
};
use axum::http::Request;
use tower_http::services::ServeDir;
use surrealdb::engine::any::{Any, connect};
use surrealdb::opt::auth::Root;
use surrealdb::{Error as SurrealError, Surreal};
use std::sync::Arc;
use serde_json::Value;
use std::time::Duration;
use tokio::time::sleep;
use serde::{Deserialize, Serialize};
use jsonwebtoken::{encode, decode, Header, Validation, EncodingKey, DecodingKey};


struct AppState {
    db: Surreal<Any>,
}


#[derive(Debug, Serialize, Deserialize)]
struct User {
    username: String,
    password: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: String,
    exp: usize,
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


async fn login(Json(user): Json<User>) -> Result<String, (StatusCode, String)> {
    // 簡単な例として、ハードコードされたユーザー情報を使用
    if user.username == "admin" && user.password == "password" {
        let claims = Claims {
            sub: user.username,
            exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp() as usize,
        };
        let token = encode(&Header::default(), &claims, &EncodingKey::from_secret("secret".as_ref()))
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        Ok(token)
    } else {
        Err((StatusCode::UNAUTHORIZED, "Invalid username or password".to_string()))
    }
}



async fn secret_page() -> impl IntoResponse {
    let html = tokio::fs::read_to_string("secret/secret.html").await.unwrap();
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html")
        .body(html)
        .unwrap()
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




async fn auth_middleware(
    req: Request<Body>,
    next: Next,
) -> Result<Response, (StatusCode, String)> {
    let token = req
        .headers()
        .get("Authorization")
        .and_then(|auth_header| auth_header.to_str().ok())
        .and_then(|auth_value| {
            if auth_value.starts_with("Bearer ") {
                Some(auth_value[7..].to_string())
            } else {
                None
            }
        })
        .or_else(|| {
            req.uri()
                .query()
                .and_then(|q| url::form_urlencoded::parse(q.as_bytes())
                    .find(|(key, _)| key == "token")
                    .map(|(_, value)| value.to_string()))
        });

    if let Some(token) = token {
        let key = b"secret";
        let validation = Validation::default();
        match decode::<Claims>(&token, &DecodingKey::from_secret(key), &validation) {
            Ok(_claims) => Ok(next.run(req).await),
            Err(_) => Err((StatusCode::UNAUTHORIZED, "Invalid token".to_string())),
        }
    } else {
        Err((StatusCode::UNAUTHORIZED, "No token provided".to_string()))
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
		.route("/api/login", post(login))
		.route("/api/data", get(get_data).route_layer(middleware::from_fn(auth_middleware)))
		.route("/secret", get(secret_page).route_layer(middleware::from_fn(auth_middleware)))
		.nest_service("/", ServeDir::new("assets"))
		.with_state(state);


    println!("==== 04 ====");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3456").await.unwrap();
    let addr = listener.local_addr().unwrap();
    println!("Listening on {}", addr);
    axum::serve(listener, app).await.unwrap();
}

