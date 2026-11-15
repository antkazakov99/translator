$ErrorActionPreference = 'Stop'
# Снимок групп языков из IANA: библиотека language-tags не предоставляет Scope.
$source = 'https://www.iana.org/assignments/language-subtag-registry/language-subtag-registry'
$registry = (Invoke-WebRequest -Uri $source).Content
if ($registry -is [byte[]]) { $registry = [Text.Encoding]::UTF8.GetString($registry) }
$date = [regex]::Match($registry, '(?m)^File-Date: (\S+)').Groups[1].Value
$codes = @($registry -split '%%' | ForEach-Object {
    if ($_ -match '(?m)^Type: language\r?$' -and $_ -match '(?m)^Scope: collection\r?$') {
        [regex]::Match($_, '(?m)^Subtag: (\S+)').Groups[1].Value
    }
} | Sort-Object -Unique)
if (!$date -or $codes.Count -lt 100 -or $codes -notcontains 'grk') {
    throw 'Не удалось прочитать группы языков из реестра IANA'
}
$output = Join-Path $PSScriptRoot '../src/name_rules/language-collections.txt'
$text = @("# Source: $source", "# File-Date: $date") + $codes
[IO.File]::WriteAllText($output, ($text -join "`n") + "`n", [Text.UTF8Encoding]::new($false))
Write-Output "Сохранено групп языков: $($codes.Count); реестр от $date"
