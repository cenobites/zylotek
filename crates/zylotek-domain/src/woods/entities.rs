use std::{fmt, str::FromStr};

use uuid::Uuid;

use crate::woods::{
    errors::WoodError,
    events::WoodEvent,
    value_objects::{ScientificName, WoodDetails, WoodName},
};

/// Aggregate root: a Brazilian wood species and its technological properties.
#[derive(Debug, Clone)]
pub struct Wood {
    id: WoodId,
    name: WoodName,
    scientific_name: ScientificName,
    details: WoodDetails,
}

impl Wood {
    pub fn create(
        name: WoodName,
        scientific_name: ScientificName,
        details: WoodDetails,
    ) -> (Self, WoodEvent) {
        let wood = Self {
            id: WoodId::new(),
            name,
            scientific_name,
            details,
        };

        let event = WoodEvent::WoodCreated { wood_id: wood.id };

        (wood, event)
    }

    /// Rebuilds a wood from storage without emitting events.
    pub fn restore(
        id: WoodId,
        name: WoodName,
        scientific_name: ScientificName,
        details: WoodDetails,
    ) -> Self {
        Self {
            id,
            name,
            scientific_name,
            details,
        }
    }

    pub fn update(
        &mut self,
        name: WoodName,
        scientific_name: ScientificName,
        details: WoodDetails,
    ) -> WoodEvent {
        self.name = name;
        self.scientific_name = scientific_name;
        self.details = details;

        WoodEvent::WoodUpdated { wood_id: self.id }
    }

    pub fn delete(self) -> WoodEvent {
        WoodEvent::WoodDeleted { wood_id: self.id }
    }

    pub fn id(&self) -> WoodId {
        self.id
    }

    pub fn name(&self) -> &WoodName {
        &self.name
    }

    pub fn scientific_name(&self) -> &ScientificName {
        &self.scientific_name
    }

    pub fn details(&self) -> &WoodDetails {
        &self.details
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WoodId(Uuid);

impl WoodId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for WoodId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for WoodId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for WoodId {
    type Err = WoodError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value)
            .map(Self)
            .map_err(|_| WoodError::InvalidId(value.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::full_details;

    fn names() -> (WoodName, ScientificName) {
        (
            WoodName::new("Muiracatiara").unwrap(),
            ScientificName::new("Astronium lecointei").unwrap(),
        )
    }

    #[test]
    fn create_assigns_an_id_and_emits_wood_created() {
        let (name, scientific_name) = names();

        let (wood, event) = Wood::create(name.clone(), scientific_name.clone(), full_details());

        assert!(matches!(event, WoodEvent::WoodCreated { wood_id } if wood_id == wood.id()));
        assert_eq!(wood.name(), &name);
        assert_eq!(wood.scientific_name(), &scientific_name);
        assert_eq!(wood.details(), &full_details());
    }

    #[test]
    fn create_gives_every_wood_a_distinct_id() {
        let (name, scientific_name) = names();

        let (first, _) = Wood::create(
            name.clone(),
            scientific_name.clone(),
            WoodDetails::default(),
        );
        let (second, _) = Wood::create(name, scientific_name, WoodDetails::default());

        assert_ne!(first.id(), second.id());
    }

    #[test]
    fn restore_keeps_the_given_id() {
        let (name, scientific_name) = names();
        let id = WoodId::new();

        let wood = Wood::restore(id, name, scientific_name, WoodDetails::default());

        assert_eq!(wood.id(), id);
    }

    #[test]
    fn update_replaces_the_state_and_emits_wood_updated() {
        let (name, scientific_name) = names();
        let (mut wood, _) = Wood::create(name, scientific_name, WoodDetails::default());
        let id = wood.id();
        let new_name = WoodName::new("Aroeira").unwrap();
        let new_scientific_name = ScientificName::new("Myracrodruon urundeuva").unwrap();

        let event = wood.update(
            new_name.clone(),
            new_scientific_name.clone(),
            full_details(),
        );

        assert!(matches!(event, WoodEvent::WoodUpdated { wood_id } if wood_id == id));
        assert_eq!(wood.id(), id);
        assert_eq!(wood.name(), &new_name);
        assert_eq!(wood.scientific_name(), &new_scientific_name);
        assert_eq!(wood.details(), &full_details());
    }

    #[test]
    fn delete_emits_wood_deleted() {
        let (name, scientific_name) = names();
        let (wood, _) = Wood::create(name, scientific_name, WoodDetails::default());
        let id = wood.id();

        assert!(matches!(wood.delete(), WoodEvent::WoodDeleted { wood_id } if wood_id == id));
    }

    #[test]
    fn wood_is_cloneable_and_debuggable() {
        let (name, scientific_name) = names();
        let (wood, event) = Wood::create(name, scientific_name, WoodDetails::default());

        assert_eq!(wood.clone().id(), wood.id());
        assert!(format!("{wood:?}").contains("Muiracatiara"));
        assert!(format!("{:?}", event.clone()).contains("WoodCreated"));
    }

    #[test]
    fn wood_id_round_trips_through_its_string_form() {
        let id = WoodId::default();

        let parsed: WoodId = id.to_string().parse().unwrap();

        assert_eq!(parsed, id);
        assert_eq!(WoodId::from_uuid(id.as_uuid()), id);
    }

    #[test]
    fn wood_id_rejects_invalid_strings() {
        let error = "not-a-uuid".parse::<WoodId>().unwrap_err();

        assert!(matches!(&error, WoodError::InvalidId(value) if value == "not-a-uuid"));
        assert_eq!(error.to_string(), "invalid wood id 'not-a-uuid'");
    }
}
