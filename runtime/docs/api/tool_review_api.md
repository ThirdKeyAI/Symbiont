# Tool Review Workflow API Specification

## Overview

The Tool Review Workflow API provides endpoints for managing the AI-driven tool review and signing process within the Symbiont platform. This API enables automated security analysis, human review coordination, and digital signing of MCP tools.

## Base URL

```
https://api.symbiont.platform/v1/tool-review
```

## Authentication

All API requests require authentication using Bearer tokens:

```http
Authorization: Bearer <token>
```

### Scopes

- `tool:review` - Submit tools for review
- `tool:analyze` - Perform security analysis
- `tool:approve` - Approve/reject tools (human reviewers)
- `tool:sign` - Sign approved tools
- `tool:admin` - Administrative operations

## Core Resources

### 1. Review Sessions

#### Submit Tool for Review

```http
POST /sessions
```

**Request Body:**
```json
{
  "tool": {
    "name": "example-tool",
    "description": "Example MCP tool",
    "schema": {
      "type": "object",
      "properties": {
        "input": {
          "type": "string",
          "description": "Tool input parameter"
        }
      },
      "required": ["input"]
    },
    "provider": {
      "name": "example-provider",
      "public_key_url": "https://example.com/pubkey.pem"
    }
  },
  "submitted_by": "user@example.com",
  "priority": "normal"
}
```

**Response (201 Created):**
```json
{
  "review_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "pending_review",
  "submitted_at": "2025-01-07T12:00:00Z",
  "estimated_completion": "2025-01-07T12:30:00Z"
}
```

#### Get Review Status

```http
GET /sessions/{review_id}
```

**Response (200 OK):**
```json
{
  "review_id": "550e8400-e29b-41d4-a716-446655440000",
  "tool": {
    "name": "example-tool",
    "description": "Example MCP tool"
  },
  "state": {
    "type": "awaiting_human_review",
    "analysis_id": "123e4567-e89b-12d3-a456-426614174000",
    "analysis_completed_at": "2025-01-07T12:15:00Z",
    "critical_findings": [
      {
        "finding_id": "INJECTION_1",
        "severity": "high",
        "category": "schema_injection",
        "title": "Potential Injection Vulnerability",
        "description": "Tool may be vulnerable to injection attacks",
        "confidence": 0.8,
        "remediation_suggestion": "Implement input validation"
      }
    ],
    "risk_score": 0.75,
    "ai_recommendation": {
      "type": "requires_human_judgment",
      "reasoning": "High-risk findings require manual review"
    }
  },
  "security_analysis": {
    "analysis_id": "123e4567-e89b-12d3-a456-426614174000",
    "risk_score": 0.75,
    "confidence_score": 0.82,
    "findings_count": 3,
    "processing_time_ms": 2500
  },
  "audit_trail": [
    {
      "event_type": "tool_submitted",
      "timestamp": "2025-01-07T12:00:00Z",
      "actor": "user@example.com"
    },
    {
      "event_type": "analysis_started",
      "timestamp": "2025-01-07T12:01:00Z",
      "actor": "ai-analyzer-v1.0"
    }
  ],
  "created_at": "2025-01-07T12:00:00Z",
  "updated_at": "2025-01-07T12:15:00Z"
}
```

#### List Review Sessions

```http
GET /sessions?status={status}&page={page}&limit={limit}
```

**Query Parameters:**
- `status` - Filter by review status (optional)
- `page` - Page number (default: 1)
- `limit` - Items per page (default: 20, max: 100)
- `submitted_by` - Filter by submitter (optional)
- `risk_level` - Filter by risk level (optional)

**Response (200 OK):**
```json
{
  "sessions": [
    {
      "review_id": "550e8400-e29b-41d4-a716-446655440000",
      "tool_name": "example-tool",
      "status": "awaiting_human_review",
      "risk_score": 0.75,
      "submitted_at": "2025-01-07T12:00:00Z",
      "submitted_by": "user@example.com"
    }
  ],
  "pagination": {
    "page": 1,
    "limit": 20,
    "total": 45,
    "total_pages": 3
  }
}
```

### 2. Security Analysis

#### Get Analysis Details

```http
GET /analysis/{analysis_id}
```

**Response (200 OK):**
```json
{
  "analysis_id": "123e4567-e89b-12d3-a456-426614174000",
  "tool_id": "example-tool",
  "analyzed_at": "2025-01-07T12:15:00Z",
  "analyzer_version": "ai-security-analyzer-v1.0",
  "risk_score": 0.75,
  "confidence_score": 0.82,
  "findings": [
    {
      "finding_id": "INJECTION_1",
      "severity": "high",
      "category": "schema_injection",
      "title": "Potential Injection Vulnerability",
      "description": "Tool may be vulnerable to injection attacks based on schema analysis",
      "location": "Query 1: Pattern analysis",
      "confidence": 0.8,
      "remediation_suggestion": "Implement input validation and sanitization",
      "cve_references": []
    }
  ],
  "recommendations": [
    "HIGH RISK: 1 high-severity issues found. Requires thorough review.",
    "Remediation: Implement input validation and sanitization"
  ],
  "analysis_metadata": {
    "processing_time_ms": 2500,
    "rag_queries_performed": 5,
    "knowledge_sources_consulted": [
      "vulnerability_patterns",
      "malicious_code_signatures"
    ],
    "patterns_matched": [
      "unvalidated_string_input",
      "command_parameter"
    ],
    "false_positive_likelihood": 0.1
  }
}
```

#### Trigger Re-analysis

```http
POST /analysis/{review_id}/reanalyze
```

**Request Body:**
```json
{
  "reason": "Updated security patterns",
  "analyzer_config": {
    "confidence_threshold": 0.7,
    "include_low_severity": false
  }
}
```

**Response (202 Accepted):**
```json
{
  "analysis_id": "789e0123-e89b-12d3-a456-426614174000",
  "status": "in_progress",
  "estimated_completion": "2025-01-07T13:15:00Z"
}
```

### 3. Human Review

#### Get Review Queue

```http
GET /review/queue
```

**Response (200 OK):**
```json
{
  "pending_reviews": [
    {
      "review_id": "550e8400-e29b-41d4-a716-446655440000",
      "tool_name": "example-tool",
      "provider": "example-provider",
      "risk_score": 0.75,
      "critical_findings_count": 1,
      "high_findings_count": 2,
      "ai_recommendation": "requires_human_judgment",
      "priority_score": 85,
      "submitted_at": "2025-01-07T12:00:00Z",
      "time_in_queue": "PT15M"
    }
  ],
  "queue_stats": {
    "total_pending": 12,
    "high_priority": 3,
    "avg_wait_time": "PT25M"
  }
}
```

#### Submit Human Decision

```http
POST /review/{review_id}/decision
```

**Request Body:**
```json
{
  "decision": "approve",
  "reasoning": "Tool appears safe after manual review. Security findings are false positives.",
  "operator_id": "reviewer@example.com",
  "time_spent_seconds": 300,
  "additional_notes": "Validated input sanitization implementation"
}
```

**Response (200 OK):**
```json
{
  "decision_id": "dec_123456789",
  "review_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "approved",
  "next_action": "signing",
  "decided_at": "2025-01-07T12:30:00Z"
}
```

### 4. Tool Signing

#### Get Signing Status

```http
GET /signing/{review_id}
```

**Response (200 OK):**
```json
{
  "review_id": "550e8400-e29b-41d4-a716-446655440000",
  "signing_status": "completed",
  "signature_info": {
    "signature": "MEUCIQDXvW...",
    "algorithm": "Ed25519",
    "public_key_url": "https://keys.symbiont.platform/signing-key.pem",
    "signed_at": "2025-01-07T12:35:00Z",
    "expires_at": "2026-01-07T12:35:00Z"
  },
  "signed_schema": {
    "original_schema": {...},
    "signature": "MEUCIQDXvW...",
    "metadata": {
      "signed_by": "symbiont-platform",
      "review_id": "550e8400-e29b-41d4-a716-446655440000"
    }
  }
}
```

#### Download Signed Tool

```http
GET /signing/{review_id}/download
```

**Response (200 OK):**
```json
{
  "tool": {
    "name": "example-tool",
    "description": "Example MCP tool",
    "schema": {...},
    "provider": {...},
    "verification_status": "signed",
    "signature_info": {
      "signature": "MEUCIQDXvW...",
      "algorithm": "Ed25519",
      "public_key_url": "https://keys.symbiont.platform/signing-key.pem",
      "signed_at": "2025-01-07T12:35:00Z"
    }
  }
}
```

### 5. Statistics and Monitoring

#### Get Workflow Statistics

```http
GET /stats
```

**Response (200 OK):**
```json
{
  "overall": {
    "total_reviews": 1247,
    "approved_tools": 1089,
    "rejected_tools": 158,
    "signed_tools": 1089,
    "avg_analysis_time_ms": 2341,
    "avg_human_review_time_ms": 450000,
    "auto_approval_rate": 0.65,
    "false_positive_rate": 0.08
  },
  "current_queue": {
    "pending_analysis": 5,
    "awaiting_human_review": 12,
    "pending_signing": 3
  },
  "top_security_categories": [
    {
      "category": "unvalidated_input",
      "count": 156
    },
    {
      "category": "privilege_escalation",
      "count": 89
    }
  ],
  "time_period": {
    "start": "2025-01-01T00:00:00Z",
    "end": "2025-01-07T12:35:00Z"
  }
}
```

## Error Handling

### Standard Error Response

```json
{
  "error": {
    "code": "VALIDATION_ERROR",
    "message": "Invalid tool schema format",
    "details": {
      "field": "schema.properties",
      "reason": "Missing required property"
    },
    "request_id": "req_123456789",
    "timestamp": "2025-01-07T12:35:00Z"
  }
}
```

### Error Codes

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `INVALID_REQUEST` | 400 | Malformed request body |
| `VALIDATION_ERROR` | 400 | Request validation failed |
| `UNAUTHORIZED` | 401 | Invalid or missing authentication |
| `FORBIDDEN` | 403 | Insufficient permissions |
| `NOT_FOUND` | 404 | Resource not found |
| `CONFLICT` | 409 | Resource conflict |
| `RATE_LIMITED` | 429 | Rate limit exceeded |
| `ANALYSIS_FAILED` | 500 | Security analysis error |
| `SIGNING_FAILED` | 500 | Tool signing error |
| `INTERNAL_ERROR` | 500 | Unexpected server error |

## Rate Limiting

API requests are rate limited per API key:

- **Submit tool**: 10 requests/minute
- **Get status**: 100 requests/minute  
- **Human decisions**: 30 requests/minute
- **Other endpoints**: 60 requests/minute

Rate limit headers:
```http
X-RateLimit-Limit: 60
X-RateLimit-Remaining: 45
X-RateLimit-Reset: 1641556800
```

## Webhooks

Configure webhooks to receive real-time notifications:

### Webhook Events

- `tool.submitted` - Tool submitted for review
- `analysis.completed` - Security analysis finished
- `review.required` - Human review needed
- `tool.approved` - Tool approved by human reviewer
- `tool.rejected` - Tool rejected
- `tool.signed` - Tool successfully signed
- `signing.failed` - Tool signing failed

### Webhook Payload

```json
{
  "event": "tool.approved",
  "data": {
    "review_id": "550e8400-e29b-41d4-a716-446655440000",
    "tool_name": "example-tool",
    "approved_by": "reviewer@example.com",
    "approved_at": "2025-01-07T12:30:00Z"
  },
  "timestamp": "2025-01-07T12:30:05Z",
  "webhook_id": "wh_123456789"
}
```

## SDK Examples

### Python SDK

```python
from symbiont_client import ToolReviewClient

client = ToolReviewClient(
    api_key="your_api_key",
    base_url="https://api.symbiont.platform/v1/tool-review"
)

# Submit tool for review
response = client.submit_tool({
    "tool": {
        "name": "my-tool",
        "description": "My custom tool",
        "schema": {...}
    },
    "submitted_by": "user@example.com"
})

review_id = response["review_id"]

# Check status
status = client.get_review_status(review_id)
print(f"Status: {status['state']['type']}")

# Wait for completion
result = client.wait_for_completion(review_id, timeout=300)
if result["state"]["type"] == "signed":
    signed_tool = client.download_signed_tool(review_id)
```

### JavaScript SDK

```javascript
import { ToolReviewClient } from '@symbiont/tool-review-sdk';

const client = new ToolReviewClient({
  apiKey: 'your_api_key',
  baseUrl: 'https://api.symbiont.platform/v1/tool-review'
});

// Submit and track tool review
async function reviewTool(tool) {
  const submission = await client.submitTool({
    tool,
    submitted_by: 'user@example.com'
  });
  
  console.log(`Review ID: ${submission.review_id}`);
  
  // Poll for completion
  const result = await client.waitForCompletion(submission.review_id);
  
  if (result.state.type === 'signed') {
    const signedTool = await client.downloadSignedTool(submission.review_id);
    return signedTool;
  } else {
    throw new Error(`Review failed: ${result.state.type}`);
  }
}
```

## OpenAPI Specification

The complete OpenAPI 3.0 specification is available at:
```
https://api.symbiont.platform/v1/tool-review/openapi.json
```

## Support

For API support and questions:
- Documentation: https://docs.symbiont.platform/api/tool-review
- Support: api-support@symbiont.platform
- Status Page: https://status.symbiont.platform