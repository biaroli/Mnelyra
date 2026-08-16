use crate::data::DataStore;
use crate::error::AppResult;

pub struct SecretStore;

impl SecretStore {
    pub fn get_or_create_app(scope: &str, item_id: &str) -> AppResult<String> {
        if let Some(value) = Self::get_app(scope, item_id)? {
            return Ok(value);
        }
        let value = random_secret();
        Self::set_app(scope, item_id, &value)?;
        Ok(value)
    }

    pub fn get_shared(key: &str) -> AppResult<Option<String>> {
        DataStore::read_file(|data| Ok(data.shared_secrets.get(key).cloned()))
    }

    pub fn get_app(scope: &str, item_id: &str) -> AppResult<Option<String>> {
        DataStore::read_file(|data| {
            Ok(data
                .app_secrets
                .get(scope)
                .and_then(|items| items.get(item_id))
                .filter(|value| !value.is_empty())
                .cloned())
        })
    }

    pub fn set_app(scope: &str, item_id: &str, value: &str) -> AppResult<()> {
        DataStore::update_file(|data| {
            data.app_secrets
                .entry(scope.to_string())
                .or_default()
                .insert(item_id.to_string(), value.to_string());
            Ok(())
        })
    }
}

fn random_secret() -> String {
    format!("{}{}", uuid::Uuid::new_v4(), uuid::Uuid::new_v4()).replace('-', "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_secret_is_non_empty() {
        assert!(random_secret().len() > 32);
    }
}
