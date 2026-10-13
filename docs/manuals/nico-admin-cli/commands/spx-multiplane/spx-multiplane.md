# `nico-admin-cli spx-multiplane`

*[Network commands](../../network.md) › **spx-multiplane***

## NAME

nico-admin-cli-spx-multiplane - Control Spectrum-X plane maintenance on
a machine

## SYNOPSIS

```text
nico-admin-cli spx-multiplane --machine-id <MACHINE_ID> maintenance-mode <on|off> --rail <0-3|all> --plane <0-7> [--reference <URL>]
       nico-admin-cli spx-multiplane --machine-id <MACHINE_ID> show
```

## DESCRIPTION

Control maintenance mode for Spectrum-X planes on a machine. Each rail
has two SuperNICs: planes 0-3 belong to one, and planes 4-7 belong to
the other. Rail R and plane P select the specified rail and plane that
are affected; its matching plane on the other SuperNIC is unchanged. All
planes start in maintenance mode. Maintenance mode on disables a plane;
maintenance mode off enables it.

## OPTIONS

`--machine-id <MACHINE_ID>`

Machine ID of the host

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
nico-admin-cli spx-multiplane --machine-id fm100psbtmb15tgh6q5duqb8ke5grng7ksd96hetbeie9nc5pvcca6eol80 maintenance-mode on --rail 2 --plane 6 --reference https://tickets.example.com/SPX-42
nico-admin-cli spx-multiplane --machine-id fm100psbtmb15tgh6q5duqb8ke5grng7ksd96hetbeie9nc5pvcca6eol80 maintenance-mode off --rail 2 --plane 6
nico-admin-cli spx-multiplane --machine-id fm100psbtmb15tgh6q5duqb8ke5grng7ksd96hetbeie9nc5pvcca6eol80 maintenance-mode on --rail all --plane 6
nico-admin-cli spx-multiplane --machine-id fm100psbtmb15tgh6q5duqb8ke5grng7ksd96hetbeie9nc5pvcca6eol80 show
```

## Subcommands

| Subcommand | Description |
|---|---|
| [`maintenance-mode`](./spx-multiplane-maintenance-mode.md) | Turn maintenance mode on (disable) or off (enable) for the selected plane |
| [`show`](./spx-multiplane-show.md) | Show requested and applied plane maintenance state |

---

**Related:** [Network commands](../../network.md) · [CLI reference index](../../README.md)
