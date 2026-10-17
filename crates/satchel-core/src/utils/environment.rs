/// Runtime overrides for local development and provider fixtures only.
pub fn debug_env(key: &str) -> Option<String> {
    #[cfg(debug_assertions)]
    {
        std::env::var(key).ok()
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = key;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_overrides_follow_the_build_mode() {
        const KEY: &str = "SATCHEL_TEST_RELEASE_ENV_POLICY";
        std::env::set_var(KEY, "fixture");
        assert_eq!(
            debug_env(KEY),
            cfg!(debug_assertions).then(|| "fixture".to_string())
        );
        std::env::remove_var(KEY);
    }
}
