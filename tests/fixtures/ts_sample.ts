import { Request, Response } from "express";

/**
 * User record returned by the API.
 */
export interface UserRecord {
  id: string;
  email: string;
}

export type Role = "admin" | "member";

// Regular line comment to strip in minimal mode.
function Injectable() {
  return function (_target: unknown) {};
}

function Get(_path: string) {
  return function (_target: unknown, _key: string) {};
}

/**
 * Controller handling user endpoints.
 */
@Injectable()
export class UserController {
  private readonly pattern = /\{[a-z]+\}\/\//g;

  @Get("/users")
  async listUsers(_req: Request, _res: Response): Promise<UserRecord[]> {
    const sample = "/* not a comment */ { }";
    if (!this.pattern.test(sample)) {
      return [];
    }
    return [{ id: "1", email: "a@example.com" }];
  }
}

export async function formatUser(user: UserRecord): Promise<string> {
  // Trailing work
  return `${user.id}:${user.email}`;
}
