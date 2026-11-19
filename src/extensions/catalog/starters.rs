use super::{Page, skill_item};

// In-app entry points to public repositories, available without a search API or account.
// Explicit paths avoid guessing which skill a repository's display name refers to.
const SKILLS: &[(&str, &str, &str, &str)] = &[
    (
        "anthropics/skills",
        "docx",
        "skills/docx",
        "Microsoft Word / DOCX",
    ),
    (
        "anthropics/skills",
        "xlsx",
        "skills/xlsx",
        "Excel / spreadsheets",
    ),
    (
        "anthropics/skills",
        "pptx",
        "skills/pptx",
        "PowerPoint / presentations",
    ),
    (
        "anthropics/skills",
        "webapp-testing",
        "skills/webapp-testing",
        "Playwright / web app testing",
    ),
    (
        "vercel-labs/agent-skills",
        "vercel-react-best-practices",
        "skills/react-best-practices",
        "React / Next.js",
    ),
    (
        "vercel-labs/agent-skills",
        "deploy-to-vercel",
        "skills/deploy-to-vercel",
        "Vercel deployment",
    ),
    (
        "vercel-labs/skills",
        "find-skills",
        "skills/find-skills",
        "Skill discovery",
    ),
    (
        "openai/plugins",
        "ios-app-intents",
        "plugins/build-ios-apps/skills/ios-app-intents",
        "Apple App Intents",
    ),
    (
        "microsoft/skills",
        "azure-advisor-cost-optimization",
        ".github/plugins/azure-advisor/skills/azure-advisor-cost-optimization",
        "Azure cost optimization",
    ),
    (
        "obra/superpowers",
        "test-driven-development",
        "skills/test-driven-development",
        "Test-driven development",
    ),
    (
        "obra/superpowers",
        "systematic-debugging",
        "skills/systematic-debugging",
        "Debugging workflow",
    ),
    (
        "remotion-dev/skills",
        "remotion-best-practices",
        "skills/remotion-best-practices",
        "Video creation with React",
    ),
];

pub(super) fn github(query: &str) -> Page {
    let query = query.to_lowercase();
    let items = SKILLS
        .iter()
        .filter(|(repo, name, _, description)| {
            [*repo, *name, *description]
                .iter()
                .any(|value| value.to_lowercase().contains(&query))
        })
        .map(|(repo, name, path, description)| skill_item(repo, name, Some(path), description))
        .collect();
    Page {
        items,
        ..Page::default()
    }
}
