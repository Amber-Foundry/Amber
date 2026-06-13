#!/bin/bash
sed -i 's/className="chat-bubble-edit-btn cancel"/className="chat-bubble-edit-btn cancel"\n                  aria-label="Cancel edit"/g' ui/components/ChatPanel.tsx
sed -i 's/className="chat-bubble-edit-btn save"/className="chat-bubble-edit-btn save"\n                  aria-label="Save edit"/g' ui/components/ChatPanel.tsx
sed -i 's/className="chat-show-more-btn"/className="chat-show-more-btn"\n                  aria-label={isCollapsed ? "Show more message content" : "Show less message content"}/g' ui/components/ChatPanel.tsx
