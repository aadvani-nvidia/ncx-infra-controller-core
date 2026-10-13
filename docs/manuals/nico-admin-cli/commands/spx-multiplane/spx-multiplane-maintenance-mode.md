# `nico-admin-cli spx-multiplane maintenance-mode`

*[Network commands](../../network.md) › [spx-multiplane](./spx-multiplane.md) › **maintenance-mode***

## NAME

nico-admin-cli-spx-multiplane-maintenance-mode - Turn maintenance mode
on (disable) or off (enable) for the selected plane

## SYNOPSIS

```text
nico-admin-cli spx-multiplane --machine-id <MACHINE_ID> maintenance-mode <on|off> --rail <0-3|all> --plane <0-7> [--reference <URL>]
```

## DESCRIPTION

Turn maintenance mode on (disable) or off (enable) for the selected
plane

## OPTIONS

`--rail <RAIL>`

Rail number (0-3), or all four rails

`--plane <PLANE>`

Plane number (0-7); planes 0-3 and 4-7 belong to separate SuperNICs

`--reference <URL>`

Optional URL describing the reason for the change

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

`<MODE>`

on disables the plane; off enables it

*Possible values:*

> - on
>
> - off

## Examples

```sh
nico-admin-cli spx-multiplane --machine-id fm100psbtmb15tgh6q5duqb8ke5grng7ksd96hetbeie9nc5pvcca6eol80 maintenance-mode on --rail 2 --plane 6 --reference https://tickets.example.com/SPX-42
nico-admin-cli spx-multiplane --machine-id fm100psbtmb15tgh6q5duqb8ke5grng7ksd96hetbeie9nc5pvcca6eol80 maintenance-mode off --rail 2 --plane 6
nico-admin-cli spx-multiplane --machine-id fm100psbtmb15tgh6q5duqb8ke5grng7ksd96hetbeie9nc5pvcca6eol80 maintenance-mode on --rail all --plane 6
```

---

**Related:** [Network commands](../../network.md) · [CLI reference index](../../README.md)
