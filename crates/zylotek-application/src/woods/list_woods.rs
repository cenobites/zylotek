use std::sync::Arc;

use zylotek_domain::woods::{entities::Wood, repositories::WoodRepository};

use crate::woods::errors::WoodUseCaseError;

pub const DEFAULT_LIMIT: usize = 20;
pub const MAX_LIMIT: usize = 100;

pub struct ListWoods {
    wood_repository: Arc<dyn WoodRepository>,
}

impl ListWoods {
    pub fn new(wood_repository: Arc<dyn WoodRepository>) -> Self {
        Self { wood_repository }
    }

    pub async fn execute(&self, query: ListWoodsQuery) -> Result<WoodPage, WoodUseCaseError> {
        let offset = query.offset.unwrap_or(0);
        let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);

        let items = self
            .wood_repository
            .list(offset, limit)
            .await
            .map_err(WoodUseCaseError::Persistence)?;

        let total = self
            .wood_repository
            .count()
            .await
            .map_err(WoodUseCaseError::Persistence)?;

        Ok(WoodPage {
            items,
            total,
            offset,
            limit,
        })
    }
}

#[derive(Debug, Default)]
pub struct ListWoodsQuery {
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

pub struct WoodPage {
    pub items: Vec<Wood>,
    pub total: u64,
    pub offset: usize,
    pub limit: usize,
}

#[cfg(test)]
mod tests {
    use zylotek_domain::testing::{Operation, wood};

    use super::*;
    use crate::woods::support::Harness;

    fn seeded(count: usize) -> Harness {
        Harness::new((0..count).map(|i| wood("Madeira", &format!("Genus species{i:03}"))))
    }

    #[tokio::test]
    async fn defaults_to_the_first_twenty() {
        let harness = seeded(25);

        let page = harness
            .list
            .execute(ListWoodsQuery::default())
            .await
            .unwrap();

        assert_eq!(
            (page.offset, page.limit, page.total),
            (0, DEFAULT_LIMIT, 25)
        );
        assert_eq!(page.items.len(), DEFAULT_LIMIT);
    }

    #[tokio::test]
    async fn honours_offset_and_limit() {
        let harness = seeded(10);

        let page = harness
            .list
            .execute(ListWoodsQuery {
                offset: Some(8),
                limit: Some(5),
            })
            .await
            .unwrap();

        assert_eq!((page.offset, page.limit, page.total), (8, 5, 10));
        assert_eq!(page.items.len(), 2);
    }

    #[tokio::test]
    async fn clamps_the_limit_to_the_allowed_range() {
        let harness = seeded(3);

        let too_small = harness
            .list
            .execute(ListWoodsQuery {
                offset: None,
                limit: Some(0),
            })
            .await
            .unwrap();
        let too_large = harness
            .list
            .execute(ListWoodsQuery {
                offset: None,
                limit: Some(usize::MAX),
            })
            .await
            .unwrap();

        assert_eq!((too_small.limit, too_small.items.len()), (1, 1));
        assert_eq!((too_large.limit, too_large.items.len()), (MAX_LIMIT, 3));
    }

    #[tokio::test]
    async fn reports_list_and_count_failures_as_persistence() {
        for operation in [Operation::List, Operation::Count] {
            let harness = Harness::failing_on(operation, []);

            let error = harness
                .list
                .execute(ListWoodsQuery::default())
                .await
                .err()
                .unwrap();

            assert!(matches!(error, WoodUseCaseError::Persistence(_)));
        }
    }
}
