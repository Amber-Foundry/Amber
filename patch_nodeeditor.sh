#!/bin/bash
sed -i 's/title="Toggle interactive charts render workspace assets"/title="Toggle interactive charts render workspace assets"\n            aria-label="Toggle interactive charts"/g' ui/components/NodeEditorDetail.tsx
sed -i 's/title="Expand editor to full center canvas focus"/title="Expand editor to full center canvas focus"\n              aria-label="Expand editor"/g' ui/components/NodeEditorDetail.tsx
