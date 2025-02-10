name: Check Diff

on:
  workflow_call:
    inputs:
      pattern:
        description: Pattern to check
        required: true
        type: string
    outputs:
      changed:
        description: true if the files matching the pattern have changed
        value: ${{ jobs.check.outputs.changed }}

jobs:
  check:
    runs-on: ubuntu-latest
    outputs:
      changed: ${{ steps.check-pr.outputs.changed || steps.check-push.outputs.changed || steps.check-merge-group.outputs.changed }}
    steps:
      - uses: actions/checkout@v4
        with:
          # Important to diff against the base branch
          fetch-depth: 2
          submodules: recursive
          token: ${{ secrets.GITHUB_TOKEN }}

      - name: Check for changes (Pull Request)
        if: github.event_name == 'pull_request'
        id: check-pr
        run: |
          FILES_CHANGED=$(git diff --name-only ${{ github.event.pull_request.base.sha }} HEAD)
          if echo "$FILES_CHANGED" | grep -m 1 "${{ inputs.pattern }}"; then
            echo "changed=true" >> $GITHUB_OUTPUT
          else
            echo "changed=false" >> $GITHUB_OUTPUT
          fi

      - name: Check for changes (Push)
        if: github.event_name == 'push'
        id: check-push
        run: |
          FILES_CHANGED=$(git diff --name-only HEAD^ HEAD)
          if echo "$FILES_CHANGED" | grep -m 1 "${{ inputs.pattern }}"; then
            echo "changed=true" >> $GITHUB_OUTPUT
          else
            echo "changed=false" >> $GITHUB_OUTPUT
          fi

      - name: Check for changes (Merge Group)
        if: github.event_name == 'merge_group'
        id: check-merge-group
        run: |
          FILES_CHANGED=$(git diff --name-only ${{ github.event.merge_group.head_ref }} HEAD)
          if echo "$FILES_CHANGED" | grep -m 1 "${{ inputs.pattern }}"; then
            echo "changed=true" >> $GITHUB_OUTPUT
          else
            echo "changed=false" >> $GITHUB_OUTPUT
          fi
