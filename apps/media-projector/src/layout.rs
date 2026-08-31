#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ProjectionLayout {
    #[default]
    Flat,
    GroupByDirectory,
}

impl ProjectionLayout {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Flat => "flat",
            Self::GroupByDirectory => "group-by-directory",
        }
    }
}
