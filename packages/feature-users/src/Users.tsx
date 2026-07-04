import { For, type Component } from "solid-js";
import { userLabel, type AuthUser } from "@auth-app/shared";

interface Member extends AuthUser {
  role: string;
  status: "active" | "invited";
}

const MEMBERS: Member[] = [
  {
    id: "1",
    email: "ada@example.com",
    displayName: "Ada Lovelace",
    role: "Owner",
    status: "active",
  },
  {
    id: "2",
    email: "grace@example.com",
    displayName: "Grace Hopper",
    role: "Admin",
    status: "active",
  },
  { id: "3", email: "alan@example.com", role: "Member", status: "invited" },
];

/** Landing view of the users feature: a table of workspace members. */
export const Users: Component = () => {
  return (
    <section class="feature">
      <header class="feature__header">
        <h1 class="feature__title">Users</h1>
        <p class="feature__subtitle">People with access to this workspace.</p>
      </header>
      <div class="card">
        <table class="table">
          <thead>
            <tr>
              <th>Name</th>
              <th>Email</th>
              <th>Role</th>
              <th>Status</th>
            </tr>
          </thead>
          <tbody>
            <For each={MEMBERS}>
              {(member) => (
                <tr>
                  <td>{userLabel(member)}</td>
                  <td class="table__muted">{member.email}</td>
                  <td>{member.role}</td>
                  <td>
                    <span classList={{ badge: true, "badge--muted": member.status === "invited" }}>
                      {member.status}
                    </span>
                  </td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>
    </section>
  );
};
