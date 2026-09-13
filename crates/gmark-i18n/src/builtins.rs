// @author kongweiguang

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::locale::{BUILTIN_LANGUAGE_EN_US_ID, BUILTIN_LANGUAGE_ZH_CN_ID, DEFAULT_LANGUAGE_ID};
use crate::{I18nError, Result, TranslationBundle};

/// Metadata for a selectable UI language.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageCatalogEntry {
    pub id: String,
    pub name: String,
}

pub(crate) struct TranslationSchema {
    pub(crate) scalar_keys: BTreeSet<String>,
    pub(crate) group_keys: BTreeSet<String>,
}

pub(crate) struct BuiltinCatalog {
    bundles: BTreeMap<String, TranslationBundle>,
    entries: Vec<LanguageCatalogEntry>,
    schema: TranslationSchema,
}

impl BuiltinCatalog {
    pub(crate) fn bundle(&self, language_id: &str) -> Option<TranslationBundle> {
        self.bundles.get(language_id).cloned()
    }

    pub(crate) fn fallback_bundle(&self, language_id: &str) -> TranslationBundle {
        self.bundle(language_id)
            .or_else(|| self.bundle(DEFAULT_LANGUAGE_ID))
            .unwrap_or_else(|| panic!("built-in English catalog must exist"))
    }

    pub(crate) fn language_name(&self, language_id: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|entry| entry.id == language_id)
            .map(|entry| entry.name.as_str())
    }

    pub(crate) fn entries(&self) -> &[LanguageCatalogEntry] {
        &self.entries
    }

    pub(crate) fn schema(&self) -> &TranslationSchema {
        &self.schema
    }
}

pub(crate) fn builtin_catalog() -> &'static BuiltinCatalog {
    static CATALOG: OnceLock<BuiltinCatalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        load_builtin_catalog()
            .unwrap_or_else(|error| panic!("embedded built-in i18n catalog is invalid: {error}"))
    })
}

fn load_builtin_catalog() -> Result<BuiltinCatalog> {
    let mut root: Value = serde_json::from_str(include_str!("builtins.json")).map_err(|error| {
        I18nError::InvalidBuiltinCatalog {
            message: error.to_string(),
        }
    })?;
    merge_builtin_supplement(
        &mut root,
        include_str!("visual_accessibility.json"),
        "visual accessibility",
    )?;
    merge_builtin_supplement(
        &mut root,
        include_str!("workspace_preferences.json"),
        "workspace preferences",
    )?;
    let catalogs = root
        .get("catalogs")
        .and_then(Value::as_object)
        .ok_or_else(|| I18nError::InvalidBuiltinCatalog {
            message: "missing catalogs object".to_owned(),
        })?;
    let mut bundles = BTreeMap::new();
    for (language_id, value) in catalogs {
        bundles.insert(
            language_id.clone(),
            TranslationBundle::from_complete_value(value).map_err(|error| {
                I18nError::InvalidBuiltinCatalog {
                    message: error.to_string(),
                }
            })?,
        );
    }

    let english =
        bundles
            .get(BUILTIN_LANGUAGE_EN_US_ID)
            .ok_or_else(|| I18nError::InvalidBuiltinCatalog {
                message: "missing en-US translations".to_owned(),
            })?;
    let chinese =
        bundles
            .get(BUILTIN_LANGUAGE_ZH_CN_ID)
            .ok_or_else(|| I18nError::InvalidBuiltinCatalog {
                message: "missing zh-CN translations".to_owned(),
            })?;
    if english.root_keys() != chinese.root_keys() || english.group_keys() != chinese.group_keys() {
        return Err(I18nError::InvalidBuiltinCatalog {
            message: "built-in language key sets differ".to_owned(),
        });
    }

    let schema = TranslationSchema {
        scalar_keys: english.scalars().keys().cloned().collect(),
        group_keys: english.groups().keys().cloned().collect(),
    };
    Ok(BuiltinCatalog {
        bundles,
        // 保持原有菜单顺序：中文在前，英语在后。
        entries: vec![
            LanguageCatalogEntry {
                id: BUILTIN_LANGUAGE_ZH_CN_ID.to_owned(),
                name: "简体中文".to_owned(),
            },
            LanguageCatalogEntry {
                id: BUILTIN_LANGUAGE_EN_US_ID.to_owned(),
                name: "English".to_owned(),
            },
        ],
        schema,
    })
}

fn merge_builtin_supplement(root: &mut Value, supplement: &str, label: &str) -> Result<()> {
    let supplement: Value =
        serde_json::from_str(supplement).map_err(|error| I18nError::InvalidBuiltinCatalog {
            message: format!("invalid {label} supplement: {error}"),
        })?;
    let additions = supplement
        .get("catalogs")
        .and_then(Value::as_object)
        .ok_or_else(|| I18nError::InvalidBuiltinCatalog {
            message: format!("{label} supplement is missing catalogs"),
        })?;
    let catalogs = root
        .get_mut("catalogs")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| I18nError::InvalidBuiltinCatalog {
            message: "missing catalogs object".to_owned(),
        })?;
    for (language_id, additions) in additions {
        let additions = additions
            .as_object()
            .ok_or_else(|| I18nError::InvalidBuiltinCatalog {
                message: format!("{label} supplement '{language_id}' must be an object"),
            })?;
        let target = catalogs
            .get_mut(language_id)
            .and_then(Value::as_object_mut)
            .ok_or_else(|| I18nError::InvalidBuiltinCatalog {
                message: format!("{label} supplement has unknown language '{language_id}'"),
            })?;
        for (key, value) in additions {
            if target.insert(key.clone(), value.clone()).is_some() {
                return Err(I18nError::InvalidBuiltinCatalog {
                    message: format!("{label} supplement duplicates '{language_id}.{key}'"),
                });
            }
        }
    }
    Ok(())
}
