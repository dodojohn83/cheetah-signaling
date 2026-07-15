#![doc = include_str!("../README.md")]

#[cfg(test)]
mod tests {
    #[test]
    fn workspace_smoke() {
        assert_eq!(env!("CARGO_PKG_NAME"), "cheetah-signaling-testkit");
    }
}
