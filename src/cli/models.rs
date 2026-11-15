use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliStatus {
    pub pipe: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MyInfoResponse {
    #[serde(rename = "Title")]
    pub title: Option<String>,
    #[serde(rename = "RealmName")]
    pub realm_name: Option<String>,
    #[serde(rename = "Level")]
    pub level: Option<i64>,
    #[serde(rename = "EnabledCombatJobDisplayName")]
    pub job_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketPricesResponse {
    #[serde(rename = "Results")]
    pub results: Option<Vec<MarketQueryResult>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketQueryResult {
    #[serde(rename = "Query")]
    pub query: String,
    #[serde(rename = "Matched")]
    pub matched: bool,
    #[serde(rename = "Rows")]
    pub rows: Vec<MarketRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketRow {
    #[serde(rename = "DisplayName")]
    pub display_name: String,
    #[serde(rename = "Rarity")]
    pub rarity: Option<String>,
    #[serde(rename = "Star")]
    pub star: Option<i64>,
    #[serde(rename = "Season")]
    pub season: Option<String>,
    #[serde(rename = "MainAttributeRange")]
    pub main_attribute_range: Option<String>,
    #[serde(rename = "Favorite")]
    pub favorite: Option<bool>,
    #[serde(rename = "InStock")]
    pub in_stock: bool,
    #[serde(rename = "MinUnitPrice")]
    pub min_unit_price: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CraftableItemsResponse {
    #[serde(rename = "craftingUnlocked")]
    pub crafting_unlocked: bool,
    pub items: Vec<CraftItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CraftItem {
    #[serde(rename = "DisplayName")]
    pub display_name: String,
    #[serde(rename = "Craftable")]
    pub craftable: bool,
    #[serde(rename = "ProducedPerCraft")]
    pub produced_per_craft: i64,
    #[serde(rename = "SuccessRate")]
    pub success_rate: f64,
    #[serde(rename = "CriticalSuccessRate")]
    pub critical_success_rate: f64,
    #[serde(rename = "Ingredients")]
    pub ingredients: Vec<CraftIngredient>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CraftIngredient {
    #[serde(rename = "DisplayName")]
    pub display_name: String,
    #[serde(rename = "Required")]
    pub required: i64,
    #[serde(rename = "Owned")]
    pub owned: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnedItem {
    #[serde(rename = "Location")]
    pub location: String, // "inventory", "character_storage", "account_storage"
    #[serde(rename = "DisplayName")]
    pub display_name: String,
    #[serde(rename = "Count")]
    pub count: i64,
    #[serde(rename = "IsLocked", default)]
    pub is_locked: bool,
}
