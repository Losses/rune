use std::collections::HashMap;

use crate::chunking::{ChunkFkMapping, DataChunk};

/// Deep merge fk_mappings from multiple DataChunks
///
/// # Arguments
/// * `chunks` - Vector of DataChunk instances
///
/// # Returns
/// * Merged ChunkFkMapping
pub fn merge_fk_mappings(chunks: &Vec<DataChunk>) -> ChunkFkMapping {
    let mut merged_mapping: ChunkFkMapping = HashMap::new();

    for chunk in chunks {
        if let Some(ref fk_mapping) = chunk.fk_mappings {
            // Iterate through each foreign key column in current chunk
            for (fk_column, parent_mappings) in fk_mapping {
                // Get or create mapping for the corresponding foreign key column
                let target_parent_mappings = merged_mapping.entry(fk_column.clone()).or_default();

                // Deep merge the inner parent_mappings
                for (parent_internal_id, sync_id_uuid) in parent_mappings {
                    target_parent_mappings.insert(parent_internal_id.clone(), sync_id_uuid.clone());
                }
            }
        }
    }

    merged_mapping
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use crate::hlc::HLC;
    use crate::utils::merge_fk_mappings;

    use super::*;

    #[test]
    fn test_merge_fk_mappings() {
        // Create test data
        let mut chunk1_fk = HashMap::new();
        let mut users_mapping1 = HashMap::new();
        users_mapping1.insert("1".to_string(), "uuid-1".to_string());
        users_mapping1.insert("2".to_string(), "uuid-2".to_string());
        chunk1_fk.insert("user_id".to_string(), users_mapping1);

        let mut chunk2_fk = HashMap::new();
        let mut users_mapping2 = HashMap::new();
        users_mapping2.insert("3".to_string(), "uuid-3".to_string());
        users_mapping2.insert("4".to_string(), "uuid-4".to_string());
        chunk2_fk.insert("user_id".to_string(), users_mapping2);

        let mut departments_mapping = HashMap::new();
        departments_mapping.insert("10".to_string(), "dept-uuid-1".to_string());
        chunk2_fk.insert("department_id".to_string(), departments_mapping);

        let node_id = HLC::new(Uuid::new_v4());
        // Create mock DataChunk instances (only focusing on fk_mappings field)
        let chunks = vec![
            DataChunk {
                start_hlc: node_id.clone(),
                end_hlc: node_id.clone(),
                count: 100,
                chunk_hash: "hash1".to_string(),
                fk_mappings: Some(chunk1_fk),
            },
            DataChunk {
                start_hlc: node_id.clone(),
                end_hlc: node_id.clone(),
                count: 150,
                chunk_hash: "hash2".to_string(),
                fk_mappings: Some(chunk2_fk),
            },
            DataChunk {
                start_hlc: node_id.clone(),
                end_hlc: node_id.clone(),
                count: 50,
                chunk_hash: "hash3".to_string(),
                fk_mappings: None, // Test None case
            },
        ];

        let merged = merge_fk_mappings(&chunks);

        // Verify merge results
        assert_eq!(merged.len(), 2); // user_id and department_id

        let user_mappings = merged.get("user_id").unwrap();
        assert_eq!(user_mappings.len(), 4); // 1, 2, 3, 4
        assert_eq!(user_mappings.get("1"), Some(&"uuid-1".to_string()));
        assert_eq!(user_mappings.get("2"), Some(&"uuid-2".to_string()));
        assert_eq!(user_mappings.get("3"), Some(&"uuid-3".to_string()));
        assert_eq!(user_mappings.get("4"), Some(&"uuid-4".to_string()));

        let dept_mappings = merged.get("department_id").unwrap();
        assert_eq!(dept_mappings.len(), 1);
        assert_eq!(dept_mappings.get("10"), Some(&"dept-uuid-1".to_string()));
    }
}
