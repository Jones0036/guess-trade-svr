use std::{sync::{Mutex, Arc}, collections::{HashMap, BTreeMap}};

use axum::{
    routing::{get, post},
    http::StatusCode,
    Json, Router, extract::Path,
};
use axum::extract::State;

use serde::{Deserialize, Serialize};
use tower_http::{classify::ServerErrorsFailureClass, trace::TraceLayer};
use tracing::{info_span, Span};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};




#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                // axum logs rejections from built-in extractors with the `axum::rejection`
                // target, at `TRACE` level. `axum::rejection=trace` enables showing those events
                format!(
                    "{}=debug,tower_http=debug,axum::rejection=trace",
                    env!("CARGO_CRATE_NAME")
                )
                .into()
            }),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let mut settings = config::Config::default();
    settings.merge(config::File::with_name("app_config.toml")).unwrap();
    let config: AppConfig = settings.try_into().unwrap();

    let mut init_st = AppState {
        users: HashMap::new(),
        trade_start_nanos: config.trade_start_nanos,
        fee: config.fee,
        asks: BTreeMap::new()
    };
    for u in config.users.iter() {
        init_st.users.insert(u.to_owned(), UserAccount { 
            balance: config.init_balance, done_trade: false
        });
    }

    for pv in config.asks.iter() {
        init_st.asks.insert(pv.price, pv.vol);
    }

    let shared_state = Arc::new(Mutex::new(init_st));
    // let shared_state = Arc::new(AppState::from(&config));

    // build our application with a route
    let app = Router::new()
        .route("/admin/board", post(admin_board))
        .route("/users/:uname/ping", post(user_ping))
        .route("/users/:uname/check_asks", post(user_check))
        .route("/users/:uname/place_bid/:price", post(user_bid))
        .with_state(shared_state)
        .layer(TraceLayer::new_for_http());

    let svr_addr = std::env::var("SVR_ADDR").unwrap();
    let listener: tokio::net::TcpListener = tokio::net::TcpListener::bind(svr_addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
#[derive(Debug, Deserialize, Serialize)]
struct PriceVol {
    pub price: i64,
    pub vol: i64
}
 
#[derive(Debug, Deserialize, Serialize)]
struct AppConfig {
    pub users: Vec<String>,
    pub trade_start_nanos: i64,
    pub init_balance: i64,
    pub fee: i64,
    pub asks: Vec<PriceVol>
}


#[derive(Debug)]
struct AppState {
    pub users: HashMap<String, UserAccount>,
    pub trade_start_nanos: i64,
    pub fee: i64,
    pub asks: BTreeMap<i64, i64>
}


fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as i64

}
async fn admin_board(
    State(state): State<Arc<Mutex<AppState>>>,
) -> (StatusCode, Json<BoardResult>) {
    let g = state.lock().unwrap();
    let mut res = BoardResult {
        done_users: Vec::new(),
        running_users: Vec::new()
    };

    for (u, ua) in g.users.iter() {
        if ua.done_trade {
            res.done_users.push((u.to_owned(), ua.clone()));
        } else {
            res.running_users.push((u.to_owned(), ua.clone()));
        }
    }

    res.done_users.sort_by_key(|(u, ua)| - ua.balance);
    res.running_users.sort_by_key(|(u, ua)| - ua.balance);

    (StatusCode::OK, Json(res))
}


async fn user_bid(
    Path((uname, price)): Path<(String, i64)>,
    State(state): State<Arc<Mutex<AppState>>>,
) -> (StatusCode, Json<BidResult>) {
    let mut g = state.lock().unwrap();
    let fee = g.fee;
    let start_ts= g.trade_start_nanos;
    let now = now();
    let balance = {
        if g.users.get(&uname).is_none() {
            return (StatusCode::NOT_FOUND, Json(BidResult::default()));
        }

        let mut res = BidResult::default();
        let ua = g.users.get_mut(&uname).unwrap();
        if ua.balance < fee {
            return (StatusCode::FORBIDDEN, Json(res));
        }
        ua.balance -= fee;
        if now < start_ts {
            return (StatusCode::FORBIDDEN, Json(res));
        }
        if ua.done_trade {
            return (StatusCode::FORBIDDEN, Json(res));
        }

        ua.balance
    };

    let mut res = BidResult::default();
    match g.asks.entry(price) {
        std::collections::btree_map::Entry::Vacant(e) => {
            return (StatusCode::OK, Json(res));
        }
        std::collections::btree_map::Entry::Occupied(mut e) => {
            let v = e.get_mut();
            if *v <= 0 {
                return (StatusCode::OK, Json(res));
            }
            *v -= 1;
            if *v <= 0 {
                e.remove();
            }

            res.trade_succ = true;
        }
    }

    {
        let ua = g.users.get_mut(&uname).unwrap();
        ua.balance -= price;
        ua.done_trade = true;
    }


    (StatusCode::OK, Json(res))

}


async fn user_check(
    Path(uname): Path<String>,
    State(state): State<Arc<Mutex<AppState>>>,
) -> (StatusCode, Json<CheckResult>) {
    let mut g = state.lock().unwrap();
    let fee = g.fee;
    let start_ts= g.trade_start_nanos;
    let now = now();
    if g.users.get(&uname).is_none() {
        return (StatusCode::NOT_FOUND, Json(CheckResult::default()));
    }

    let ua = g.users.get_mut(&uname).unwrap();
    if ua.balance < fee {
        return (StatusCode::FORBIDDEN, Json(CheckResult::default()));
    }
    ua.balance -= fee;

    if now < start_ts {
        return (StatusCode::FORBIDDEN, Json(CheckResult::default()));
    }

    let res = CheckResult {
        asks: g.asks.iter().map(|(k,v)| PriceVol {price: *k, vol: *v }).collect()
    };
    (StatusCode::OK, Json(res))
}


async fn user_ping(
    Path(uname): Path<String>,
    State(state): State<Arc<Mutex<AppState>>>,
) -> (StatusCode, Json<PingResult>) {
    let mut g = state.lock().unwrap();
    let fee = g.fee;
    let start_ts= g.trade_start_nanos;
    if g.users.get(&uname).is_none() {
        return (StatusCode::NOT_FOUND, Json(PingResult::default()));
    }

    let ua = g.users.get_mut(&uname).unwrap();
    if ua.balance < fee {
        return (StatusCode::FORBIDDEN, Json(PingResult::default()));
    }
    ua.balance -= fee;

    let ping_res = PingResult{ now_nanos: now(), trade_start_nanos: start_ts, balance: ua.balance };
    (StatusCode::OK, Json(ping_res))
}

#[derive(Serialize, Default)]
struct BoardResult {
    pub done_users:  Vec<(String, UserAccount)>,
    pub running_users:  Vec<(String, UserAccount)>
}



#[derive(Serialize, Default)]
struct CheckResult {
    pub asks: Vec<PriceVol>
}


#[derive(Serialize, Default)]
struct PingResult {
    pub now_nanos: i64,
    pub trade_start_nanos: i64,

    pub balance: i64,
}

#[derive(Serialize, Default)]
struct BidResult {
    pub trade_succ: bool,
}

#[derive(Serialize, Debug, Clone)]
struct UserAccount {
    pub balance: i64,
    pub done_trade: bool
}
