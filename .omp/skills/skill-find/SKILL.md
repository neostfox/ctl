---
name: skill-find
description: >-
  Automatically discover and recommend the best skill for a given task.
  Use this when you encounter a task and need to find the right skill to handle it.
  The skill will analyze available skills and recommend the most appropriate one.
---

# Skill Finder

This skill helps you find the right skill for any task.

## When to Use

Use this skill when:
- You encounter a new task and don't know which skill to use
- You want to discover available skills for a specific domain
- You need to find the best tool for a job

## How It Works

1. **Analyze the Task**: Understand what the user wants to accomplish
2. **Search Skills**: Look through available skills in:
   - Project skills: `.omp/skills/*/SKILL.md`
   - User skills: `~/.omp/agent/skills/*/SKILL.md`
   - Bundled skills: Built-in OMP skills
3. **Match and Recommend**: Find the best skill based on:
   - Skill description and capabilities
   - Task requirements
   - Skill availability

## Usage Examples

### Example 1: Find a skill for code review
```
User: "I need to review this code for security issues"
Skill-find: Recommends "reviewer" or "security-audit" skill
```

### Example 2: Find a skill for documentation
```
User: "Help me write API documentation"
Skill-find: Recommends "docs" or "api-docs" skill
```

### Example 3: Find a skill for testing
```
User: "I need to write unit tests"
Skill-find: Recommends "test" or "testing" skill
```

## Implementation

When this skill is invoked:

1. **List Available Skills**:
   ```bash
   # Check project skills
   ls .omp/skills/*/SKILL.md
   
   # Check user skills
   ls ~/.omp/agent/skills/*/SKILL.md
   ```

2. **Read Skill Descriptions**:
   - Parse frontmatter from each SKILL.md
   - Extract name and description

3. **Match Task to Skill**:
   - Analyze task keywords
   - Match against skill descriptions
   - Rank by relevance

4. **Provide Recommendation**:
   - Show top 3 matching skills
   - Explain why each is relevant
   - Suggest how to invoke the skill

## Output Format

When recommending skills, use this format:

```
## Recommended Skills for: [Task Description]

### 1. [Skill Name]
- **Why**: [Reason this skill is relevant]
- **How**: `/skill:[skill-name]`

### 2. [Skill Name]
- **Why**: [Reason this skill is relevant]
- **How**: `/skill:[skill-name]`

### 3. [Skill Name]
- **Why**: [Reason this skill is relevant]
- **How**: `/skill:[skill-name]`
```

## Notes

- This skill does not execute other skills, it only recommends them
- Always verify skill availability before recommending
- If no suitable skill is found, suggest creating a new one
- Consider task complexity when recommending skills
