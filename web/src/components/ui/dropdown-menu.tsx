import { DropdownMenu as RadixDropdownMenu } from "radix-ui";
import type { ComponentProps, ReactNode } from "react";
import { cn } from "../../lib/utils";

export const Root = RadixDropdownMenu.Root;
export const Trigger = RadixDropdownMenu.Trigger;

export function Content({ className, ...props }: ComponentProps<typeof RadixDropdownMenu.Content>) {
  return (
    <RadixDropdownMenu.Portal>
      <RadixDropdownMenu.Content
        sideOffset={4}
        className={cn(
          "z-50 min-w-[8rem] rounded-md border border-border bg-popover p-1 text-popover-foreground shadow-md",
          className,
        )}
        {...props}
      />
    </RadixDropdownMenu.Portal>
  );
}

export function Item({
  className,
  children,
  ...props
}: ComponentProps<typeof RadixDropdownMenu.Item> & { children: ReactNode }) {
  return (
    <RadixDropdownMenu.Item
      className={cn(
        "flex cursor-default select-none items-center gap-2 rounded-sm px-2 py-1.5 text-[13px] outline-none data-[highlighted]:bg-accent data-[highlighted]:text-accent-foreground",
        className,
      )}
      {...props}
    >
      {children}
    </RadixDropdownMenu.Item>
  );
}
