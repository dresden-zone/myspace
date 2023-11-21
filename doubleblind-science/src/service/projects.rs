use std::sync::Arc;

use sea_orm::entity::EntityTrait;
use sea_orm::ActiveValue::Unchanged;
use sea_orm::Set;
use sea_orm::{ActiveModelTrait, DatabaseConnection};
use time::OffsetDateTime;
use uuid::Uuid;

use entity::{project, user};

#[derive(Clone)]
pub(crate) struct ProjectService {
  db: Arc<DatabaseConnection>,
}

impl ProjectService {
  pub(crate) fn from_db(db: Arc<DatabaseConnection>) -> ProjectService {
    ProjectService { db }
  }

  pub(crate) async fn all_projects(&self, zone_id: Uuid) -> anyhow::Result<Vec<project::Model>> {
    Ok(project::Entity::find().all(&*self.db).await?)
  }
  pub(crate) async fn get_project(
    &self,
    project_id: Uuid,
  ) -> anyhow::Result<Option<project::Model>> {
    Ok(
      project::Entity::find_by_id(project_id)
        .one(&*self.db)
        .await?,
    )
  }

  pub(crate) async fn create_project(
    &self,
    owner_uuid: Uuid,
    domain: String,
    commit: String,
    github_id: i64,
  ) -> anyhow::Result<Option<project::Model>> {
    let project_uuid = Uuid::new_v4();

    if let Some(user) = user::Entity::find_by_id(owner_uuid).one(&*self.db).await? {
      Ok(Some(
        project::ActiveModel {
          id: Set(project_uuid),
          owner: Set(owner_uuid),
          domain: Set(domain),
          commit: Set(commit),
          github_id: Set(Some(github_id)),
          created_at: Set(OffsetDateTime::now_utc()),
          last_update: Set(OffsetDateTime::now_utc()),
          trusted: Set(user.trusted),
        }
        .insert(&*self.db)
        .await?,
      ))
    } else {
      Ok(None)
    }
  }

  pub(crate) async fn delete(&self, project_id: Uuid) -> anyhow::Result<bool> {
    Ok(
      project::Entity::delete_by_id(project_id)
        .exec(&*self.db)
        .await?
        .rows_affected
        > 0,
    )
  }

  pub(crate) async fn trust_project(&self, project_id: Uuid) -> anyhow::Result<bool> {
    let found_project = project::Entity::find_by_id(project_id)
      .one(&*self.db)
      .await?;

    if let Some(project) = found_project {
      project::ActiveModel {
        id: Unchanged(project_id),
        owner: Unchanged(project.owner),
        domain: Unchanged(project.domain),
        commit: Unchanged(project.commit),
        created_at: Unchanged(project.created_at),
        github_id: Unchanged(project.github_id),
        last_update: Set(OffsetDateTime::now_utc()),
        trusted: Set(!project.trusted),
      }
      .update(&*self.db)
      .await?;
      Ok(true)
    } else {
      Ok(false)
    }
  }

  pub(crate) async fn get_available_projects(&self, user_id: Uuid) {}
}
