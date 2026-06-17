"""Unit tests for TokenJuice compression rules."""

from plugins.tokenjuice.rules import RULES, CompressionRule


class TestCompressionRule:
    def test_applies_to_matching_tag_and_size(self):
        rule = CompressionRule("git", 100, [], "test")
        assert rule.applies_to("git", 200) is True

    def test_not_applies_to_wrong_tag(self):
        rule = CompressionRule("git", 100, [], "test")
        assert rule.applies_to("docker", 200) is False

    def test_not_applies_to_small_output(self):
        rule = CompressionRule("git", 500, [], "test")
        assert rule.applies_to("git", 100) is False

    def test_not_applies_to_exact_threshold(self):
        rule = CompressionRule("git", 240, [], "test")
        assert rule.applies_to("git", 240) is False  # > not >=


class TestRulesExist:
    def test_all_expected_tags_have_rules(self):
        expected_tags = {
            "git",
            "package_install",
            "docker",
            "grep",
            "file_list",
            "test_run",
            "error_output",
            "code",
            "generic",
        }
        rule_tags = {r.tag for r in RULES}
        assert expected_tags == rule_tags, (
            f"Missing rules for: {expected_tags - rule_tags}, "
            f"Extra: {rule_tags - expected_tags}"
        )

    def test_no_empty_transforms(self):
        for rule in RULES:
            assert len(rule.transforms) > 0, (
                f"Rule '{rule.tag}' has no transforms"
            )

    def test_all_rules_have_descriptions(self):
        for rule in RULES:
            assert rule.description, f"Rule '{rule.tag}' has no description"

    def test_git_rule_starts_with_strip_ansi(self):
        git_rule = next(r for r in RULES if r.tag == "git")
        from plugins.tokenjuice.transforms import strip_ansi
        assert git_rule.transforms[0] is strip_ansi

    def test_generic_rule_is_last(self):
        assert RULES[-1].tag == "generic"
