# app-update-check Specification

## Purpose

规范 Pony Agent 应用内软件更新检测能力：前端直连 GitHub Releases API 查询最新发布版本，比对 semver 版本号，在侧边栏设置入口提供角标提醒，并在配置页通用面板中展示版本状态、构造式发布页跳转与隐私开关。

## Requirements

### Requirement: Application update check SHALL query GitHub releases and compare semver
The application SHALL check for software updates by anonymously querying GitHub Releases API (`lanhui100/pony-agent/releases/latest`) with at least 24 hours throttling between automatic checks. Release versions SHALL be parsed using strict semver format, and an update is considered available only when the candidate version is strictly newer than the current application version.

#### Scenario: Newer version available triggers update alert
- **GIVEN** an application running version `0.1.90`
- **WHEN** GitHub latest release reports `v0.2.0`
- **THEN** `hasUpdate` resolves to `true`
- **AND** the settings entry in the sidebar displays an update badge

#### Scenario: Malformed version or draft does not trigger false positive
- **GIVEN** GitHub latest release tag is malformed, draft, or not valid semver
- **WHEN** the update check completes
- **THEN** `hasUpdate` resolves to `false`
- **AND** no update alert is shown

### Requirement: Release page URL SHALL be constructed safely
The application SHALL NOT use raw `html_url` returned from the API for navigation. Instead, it SHALL construct the URL using the verified tag name: `https://github.com/lanhui100/pony-agent/releases/tag/<encoded_tag>`.

#### Scenario: Valid tag generates canonical release URL
- **GIVEN** verified release tag `v0.2.0`
- **WHEN** generating release page URL
- **THEN** the URL is `https://github.com/lanhui100/pony-agent/releases/tag/v0.2.0`
