$example = Get-ChildItem .\examples | ForEach-Object { $_.name } | fzf
Write-Host "You chose: $example"
cargo run --package $example