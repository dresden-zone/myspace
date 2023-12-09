use axum::extract::{Json, Query, State};
use axum::http::StatusCode;
use entity::project;
use rand::{distributions::Alphanumeric, Rng};
use serde::{Deserialize, Serialize};

use time::OffsetDateTime;
use tracing::{error, info};

use crate::auth::Session;
use crate::state::DoubleBlindState;

#[derive(Deserialize)]
pub(super) struct CreateProjectRequest {
  pub domain: String,
  pub github_name: String,
}

#[derive(Serialize, Deserialize)]
pub(super) struct RepoInformation {
  id: i64,
  name: String,
  full_name: String,
}

#[derive(Deserialize)]
pub(super) struct RepoSearch {
  search: String
}

#[derive(Deserialize)]
pub(super) struct WebhookRegistrationResponse {
  pub active: bool,
  pub id: i64,
}

#[derive(Serialize)]
pub(super) struct WebHookInformation {
  url: String,
  content_type: String,
  insecure_ssl: String,
  //token: String,
}

#[derive(Serialize)]
pub(super) struct WebhookRegistrationRequest {
  name: String,
  active: bool,
  events: Vec<String>,
  config: WebHookInformation,
}

#[derive(Serialize)]
pub(super) struct GithubDispatchEvent {
  event_type: String,
  //client_payload: serde_json::Value
}

#[derive(Deserialize)]
pub(super) struct GithubRepoInformation {
  id: i64,
  full_name: String,
}

#[derive(Serialize, Deserialize)]
pub(super) struct SearchItem {
  full_name: String
}
#[derive(Serialize, Deserialize)]
pub(super) struct SearchResponse {
  items: Vec<SearchItem>
}

pub(super) async fn user_projects(
  Session(session): Session,
  State(state): State<DoubleBlindState>,
) -> Json<Vec<project::Model>> {
  Json(
    match state
      .project_service
      .get_user_projects(session.user_id)
      .await
    {
      Ok(value) => value,
      Err(e) => {
        error!("error while querying projects {:?}", e);
        Vec::new()
      }
    },
  )
}

pub(super) async fn create_project(
  Session(session): Session,
  State(mut state): State<DoubleBlindState>,
  Json(data): Json<CreateProjectRequest>,
) -> Result<StatusCode, StatusCode> {
  if data.domain.len() <= 6 {
    return Err(StatusCode::BAD_REQUEST);
  }

  info!("user {} trying to create project subdomain: {} github repo: {}", &session.user_id, &data.domain, &data.github_name);

  match state
    .project_service
    .get_project_by_name_or_repo(&data.domain, &data.github_name)
    .await
  {
    Ok(Some(_found_project)) => {
      info!("project already exists name or repo");
      return Err(StatusCode::BAD_REQUEST);
    }
    Err(e) => {
      error!("error while searching for projects {:?}", e);
      return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    _ => {}
  }
  // TODO: throw away is redudant
  let user_info = match state.user_service.get_user(session.user_id).await {
    Ok(Some(user)) => user,
    Err(e) => {
      error!("while trying to fetch user {:?}", e);
      return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    _ => {
      return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
  };

  if let (
    Some(mut access_token),
    Some(access_token_expr),
    Some(_refresh_token),
    Some(_refresh_token_expr),
  ) = (
    user_info.github_access_token,
    user_info.github_access_token_expire,
    user_info.github_refresh_token,
    user_info.github_refresh_token_expire,
  ) {
    if access_token_expr < OffsetDateTime::now_utc() {
      match state
        .user_service
        .fresh_access_token(&mut state.oauth_github_client, session.user_id)
        .await
      {
        Some(new_token) => {
          info!("successfully refreshed access token!");
          access_token = new_token;
        }
        None => {
          return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
      };
    }

    let client = reqwest::Client::new();

    let secret_token: String = rand::thread_rng()
      .sample_iter(&Alphanumeric)
      .take(32)
      .map(char::from)
      .collect();

    let response = client
      .post(format!(
        "https://api.github.com/repos/{}/hooks",
        &data.github_name
      ))
      .header(reqwest::header::ACCEPT, "application/vnd.github+json")
      .header(
        reqwest::header::AUTHORIZATION,
        format!("Bearer {}", access_token.clone()),
      )
      .header("X-GitHub-Api-Version", "2022-11-28")
      .header(reqwest::header::USER_AGENT, "doubleblind-science")
      .json(&WebhookRegistrationRequest {
        name: "doubleblind-science-deploy-hook".to_string(),
        active: true,
        events: vec!["push".to_string()],
        config: WebHookInformation {
          url: "https://api.science.tanneberger.me/hooks/github".to_string(),
          content_type: "json".to_string(),
          insecure_ssl: "0".to_string(),
          //token: secret_token.clone(),
        },
      })
      .send()
      .await
      .map_err(|e| {
        error!("cannot register webhook with github {e}");
        StatusCode::INTERNAL_SERVER_ERROR
      })?;

    info!("information from github for webhook register {:?}", response);

    if response.status() != StatusCode::CREATED {
      error!("github api returned {}", response.status());
      return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    let _model = state
      .project_service
      .create_project(
        user_info.id,
        &data.domain,
        "",
        &data.github_name,
        &secret_token,
      )
      .await
      .map_err(|err| {
        error!("Error while inserting project: {:?}", err);
        StatusCode::INTERNAL_SERVER_ERROR
      })?;

    // now trigger github dispatch event so the webhook gets triggered

    let response = client
      .get(format!(
        "https://api.github.com/repos/{}/dispatches",
        &data.github_name
      ))
      .header(reqwest::header::ACCEPT, "application/vnd.github+json")
      .header(
        reqwest::header::AUTHORIZATION,
        format!("Bearer {}", access_token.clone()),
      )
      .header("X-GitHub-Api-Version", "2022-11-28")
      .header(reqwest::header::USER_AGENT, "doubleblind-science")
      .json(&GithubDispatchEvent {
        event_type: "doubleblind-science-setup".to_string(),
      })
      .send()
      .await
      .map_err(|e| {
        error!("cannot dispatch webhook event with github {e}");
        StatusCode::INTERNAL_SERVER_ERROR
      })?;

    if response.status() == reqwest::StatusCode::NO_CONTENT {
      Ok(StatusCode::OK)
    } else {
      Err(StatusCode::INTERNAL_SERVER_ERROR)
    }
  } else {
    Err(StatusCode::INTERNAL_SERVER_ERROR)
  }
}
