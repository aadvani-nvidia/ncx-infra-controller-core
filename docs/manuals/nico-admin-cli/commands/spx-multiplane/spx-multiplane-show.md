# `nico-admin-cli spx-multiplane show`

*[Network commands](../../network.md) › [spx-multiplane](./spx-multiplane.md) › **show***

## NAME

nico-admin-cli-spx-multiplane-show - Show requested and applied plane
maintenance state

## SYNOPSIS

```text
nico-admin-cli spx-multiplane --machine-id <MACHINE_ID> show
```

## DESCRIPTION

Show requested and applied plane maintenance state

## OPTIONS

`--extended`

Extended result output.

This is used by measured boot, where basic output contains just what you
probably care about, and "extended" output also dumps out all the
internal UUIDs that are used to associate instances.

`--sort-by <SORT_BY> [default: primary-id]`

Sort output by specified field

*Possible values:*

> - primary-id: Sort by the primary ID
>
> - state: Sort by state

`-h, --help`

Print help (see a summary with -h)

## Examples

```sh
nico-admin-cli spx-multiplane --machine-id fm100psbtmb15tgh6q5duqb8ke5grng7ksd96hetbeie9nc5pvcca6eol80 show
```

---

**Related:** [Network commands](../../network.md) · [CLI reference index](../../README.md)
