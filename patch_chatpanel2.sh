#!/bin/bash
sed -i 's/title="Copy message"/title="Copy message"\n              aria-label={isCopied ? "Copied message" : "Copy message"}/g' ui/components/ChatPanel.tsx
sed -i 's/title="Retry response"/title="Retry response"\n              aria-label="Retry response"/g' ui/components/ChatPanel.tsx
sed -i 's/title="Edit message"/title="Edit message"\n              aria-label="Edit message"/g' ui/components/ChatPanel.tsx
